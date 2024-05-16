use crate::{label::Label, Error};

/// A package reference, consisting of kebab-case namespace and name, e.g. `wasm-pkg:loader`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PackageRef {
    namespace: Label,
    name: Label,
}

impl PackageRef {
    /// Returns the namespace of the package.
    pub fn namespace(&self) -> &Label {
        &self.namespace
    }

    /// Returns the name of the package.
    pub fn name(&self) -> &Label {
        &self.name
    }
}

impl std::fmt::Display for PackageRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.namespace, self.name)
    }
}

impl<'a> TryFrom<&'a str> for PackageRef {
    type Error = Error;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        let Some((namespace, name)) = value.split_once(':') else {
            return Err(Error::InvalidPackageRef("missing expected ':'".into()));
        };
        Ok(Self {
            namespace: namespace.parse()?,
            name: name.parse()?,
        })
    }
}

impl std::str::FromStr for PackageRef {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.try_into()
    }
}
