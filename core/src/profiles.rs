use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type ProfileId = Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionProfile {
    pub id: ProfileId,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    #[serde(default)]
    pub remember_password: bool,
}

impl ConnectionProfile {
    pub fn new(
        name: String,
        host: String,
        port: u16,
        database: String,
        username: String,
        remember_password: bool,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            host,
            port,
            database,
            username,
            remember_password,
        }
    }
}
