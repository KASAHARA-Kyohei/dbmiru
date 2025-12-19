use dbmiru_core::{Result, profiles::ProfileId};
use keyring::Entry;

pub struct SecretStore {
    service_name: String,
}

impl SecretStore {
    pub fn new() -> Self {
        Self {
            service_name: "DbMiru".into(),
        }
    }

    pub fn read_password(&self, profile_id: ProfileId, username: &str) -> Result<Option<String>> {
        let entry = self.entry(profile_id, username)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn write_password(
        &self,
        profile_id: ProfileId,
        username: &str,
        password: &str,
    ) -> Result<()> {
        let entry = self.entry(profile_id, username)?;
        entry.set_password(password)?;
        Ok(())
    }

    pub fn delete_password(&self, profile_id: ProfileId, username: &str) -> Result<()> {
        let entry = self.entry(profile_id, username)?;
        match entry.delete_password() {
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    fn entry(&self, profile_id: ProfileId, username: &str) -> Result<Entry> {
        let account = format!("{profile_id}:{username}");
        Ok(Entry::new(&self.service_name, &account)?)
    }
}
impl Default for SecretStore {
    fn default() -> Self {
        Self::new()
    }
}
