use std::{
    fs,
    path::{Path, PathBuf},
};

use dbmiru_core::{Result, profiles::ConnectionProfile};

#[derive(Clone, Debug)]
pub struct ProfileStore {
    path: PathBuf,
}

impl ProfileStore {
    pub fn new(config_dir: &Path) -> Self {
        let path = config_dir.join("profiles.json");
        Self { path }
    }

    pub fn load(&self) -> Result<Vec<ConnectionProfile>> {
        match fs::read_to_string(&self.path) {
            Ok(contents) => {
                let profiles: Vec<ConnectionProfile> = serde_json::from_str(&contents)?;
                Ok(profiles)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn save(&self, profiles: &[ConnectionProfile]) -> Result<()> {
        let serialized = serde_json::to_string_pretty(profiles)?;
        fs::write(&self.path, serialized)?;
        Ok(())
    }
}
