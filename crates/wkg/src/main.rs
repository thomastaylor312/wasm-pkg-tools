mod package_spec;

use std::{io::Seek, path::PathBuf};

use anyhow::{ensure, Context};
use clap::{Args, Parser, Subcommand, ValueEnum};
use futures_util::TryStreamExt;
use package_spec::PackageSpec;
use tokio::io::AsyncWriteExt;
use tracing::level_filters::LevelFilter;
use wasm_pkg_loader::ClientConfig;
use wit_component::DecodedWasm;

#[derive(Parser, Debug)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Args, Debug)]
struct RegistryArgs {
    /// The registry domain to use. Overrides configuration file(s).
    #[arg(long = "registry", value_name = "DOMAIN")]
    domain: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Get a package.
    Get(GetCommand),
}

#[derive(Args, Debug)]
struct GetCommand {
    /// Output path. If this ends with a '/', a filename based on the package
    /// name, version, and format will be appended, e.g.
    /// `name-space_name@1.0.0.wasm``.
    #[arg(long, short, default_value = "./")]
    output: PathBuf,

    /// Output format. The default of "auto" detects the format based on the
    /// output filename or package contents.
    #[arg(long, value_enum, default_value = "auto")]
    format: Format,

    /// Overwrite any existing output file.
    #[arg(long)]
    overwrite: bool,

    /// The package to get, specified as <namespace>:<name> plus optional
    /// @<version>, e.g. "wasi:cli" or "wasi:http@0.2.0".
    package_spec: PackageSpec,

    #[command(flatten)]
    registry: RegistryArgs,
}

#[derive(ValueEnum, Clone, Debug, PartialEq)]
enum Format {
    Auto,
    Wasm,
    Wit,
}

impl GetCommand {
    pub async fn run(self) -> anyhow::Result<()> {
        let PackageSpec { package, version } = self.package_spec;

        let mut client = {
            let mut config = ClientConfig::default();
            config.set_default_registry("bytecodealliance.org");
            if let Some(file_config) = ClientConfig::from_default_file()? {
                config.merge_config(file_config);
            }
            if let Some(registry) = self.registry.domain {
                let namespace = package.namespace().to_string();
                tracing::debug!(namespace, registry, "overriding namespace registry");
                config.set_namespace_registry(namespace, registry);
            }
            config.to_client()
        };

        let version = match version {
            Some(ver) => ver,
            None => {
                println!("No version specified; fetching version list...");
                let versions = client.list_all_versions(&package).await?;
                tracing::trace!(?versions);
                versions
                    .into_iter()
                    .filter_map(|vi| (!vi.yanked).then_some(vi.version))
                    .max()
                    .context("No releases found")?
            }
        };

        println!("Getting {package}@{version}...");
        let release = client
            .get_release(&package, &version)
            .await
            .context("Failed to get release details")?;
        tracing::debug!(?release);

        let output_trailing_slash = self.output.as_os_str().to_string_lossy().ends_with('/');
        let parent_dir = if output_trailing_slash {
            self.output.as_path()
        } else {
            self.output
                .parent()
                .context("Failed to resolve output parent dir")?
        };

        let (tmp_file, tmp_path) =
            tempfile::NamedTempFile::with_prefix_in(".wkg-get", parent_dir)?.into_parts();
        tracing::debug!(?tmp_path);

        let mut content_stream = client.stream_content(&package, &release).await?;

        let mut file = tokio::fs::File::from_std(tmp_file);
        while let Some(chunk) = content_stream.try_next().await? {
            file.write_all(&chunk).await?;
        }

        let mut format = self.format;
        if let (Format::Auto, Some(ext)) = (&format, self.output.extension()) {
            tracing::debug!("Inferring output format from file extension {ext:?}");
            format = match ext.to_string_lossy().as_ref() {
                "wasm" => Format::Wasm,
                "wit" => Format::Wit,
                _ => {
                    println!(
                        "Couldn't infer output format from file name {:?}",
                        self.output.file_name().unwrap_or_default()
                    );
                    Format::Auto
                }
            }
        }

        let wit = if format == Format::Wasm {
            None
        } else {
            let mut file = file.into_std().await;
            file.rewind()?;
            match wit_component::decode_reader(&mut file) {
                Ok(DecodedWasm::WitPackage(resolve, pkg)) => {
                    tracing::debug!(?pkg, "decoded WIT package");
                    Some(wit_component::WitPrinter::default().print(&resolve, pkg)?)
                }
                Ok(_) => None,
                Err(err) => {
                    tracing::debug!(?err);
                    if format == Format::Wit {
                        return Err(err);
                    }
                    println!("Failed to detect package content type: {err:#}");
                    None
                }
            }
        };

        let output_path = if output_trailing_slash {
            let ext = if wit.is_some() { "wit" } else { "wasm" };
            self.output.join(format!(
                "{namespace}_{name}@{version}.{ext}",
                namespace = package.namespace(),
                name = package.name(),
            ))
        } else {
            self.output
        };
        ensure!(
            self.overwrite || !output_path.exists(),
            "{output_path:?} already exists; you can use '--overwrite' to overwrite it"
        );

        if let Some(wit) = wit {
            std::fs::write(&output_path, wit)
                .with_context(|| format!("Failed to write WIT to {output_path:?}"))?
        } else {
            tmp_path
                .persist(&output_path)
                .with_context(|| format!("Failed to persist WASM to {output_path:?}"))?
        }
        println!("Wrote '{}'", output_path.display());

        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(LevelFilter::WARN.into())
                .from_env_lossy(),
        )
        .init();

    let cli = Cli::parse();
    tracing::debug!(?cli);

    match cli.command {
        Commands::Get(cmd) => cmd.run().await,
    }
}
