//! A fresh Postgres database that several servers of one test open, as in production (with
//! `data.database_url`); only when `ORBIT_TEST_POSTGRES_URL` names a Postgres server.

use sqlx::Connection;

pub struct SharedDatabase {
    server: String,
    name: String,
    pub url: String,
}

/// An app key for every server of the database.
pub const APP_KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

impl SharedDatabase {
    pub async fn create() -> Option<Self> {
        let server = std::env::var("ORBIT_TEST_POSTGRES_URL")
            .ok()
            .filter(|url| !url.is_empty())?;
        let name = format!(
            "orbit_t_shared_{}",
            orbit_platform::Id::new_v7().to_string().replace('-', "")
        );
        let mut admin = sqlx::PgConnection::connect(&server).await.unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE {name} TEMPLATE template0 ENCODING 'UTF8' \
             LOCALE_PROVIDER builtin BUILTIN_LOCALE 'C.UTF-8' LOCALE 'C.UTF-8'"
        )))
        .execute(&mut admin)
        .await
        .unwrap();
        let (base, _) = server.rsplit_once('/').unwrap();
        let url = format!("{base}/{name}");
        Some(Self { server, name, url })
    }

    /// Drops the database; every server must be stopped.
    pub async fn drop(self) {
        let mut admin = sqlx::PgConnection::connect(&self.server).await.unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            self.name
        )))
        .execute(&mut admin)
        .await
        .unwrap();
    }
}
