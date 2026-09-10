use super::Error;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, process::Stdio};
use tokio::io::AsyncWriteExt;

/// Intentionally no Debug implementation: session material must never reach logs.
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    #[serde(flatten)]
    pub values: BTreeMap<String, String>,
}

impl Session {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let session: Self = serde_json::from_slice(bytes).map_err(|_| Error::SessionFile)?;
        for key in ["sessionid", "ds_user_id", "csrftoken"] {
            let s = session.values.get(key).ok_or(Error::SessionFile)?;
            if s.is_empty() || s.contains(['\r', '\n', ';']) {
                return Err(Error::SessionFile);
            }
        }
        if !session.values["ds_user_id"]
            .bytes()
            .all(|b| b.is_ascii_digit())
        {
            return Err(Error::SessionFile);
        }
        Ok(session)
    }
    pub async fn restore() -> Result<Self, Error> {
        let output = tokio::process::Command::new("secret-tool")
            .args(["lookup", "application", "io.github._6E6B.viewfinder"])
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|_| Error::Keyring)?;
        if !output.status.success() {
            return Err(Error::Keyring);
        }
        Self::parse(&output.stdout)
    }
    pub async fn save(&self) -> Result<(), Error> {
        let mut child = tokio::process::Command::new("secret-tool")
            .args([
                "store",
                "--label=Viewfinder session",
                "application",
                "io.github._6E6B.viewfinder",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| Error::Keyring)?;
        let data = serde_json::to_vec(self).map_err(|_| Error::Keyring)?;
        let mut stdin = child.stdin.take().ok_or(Error::Keyring)?;
        stdin.write_all(&data).await.map_err(|_| Error::Keyring)?;
        drop(stdin);
        if child.wait().await.map_err(|_| Error::Keyring)?.success() {
            Ok(())
        } else {
            Err(Error::Keyring)
        }
    }
    pub async fn forget() -> Result<(), Error> {
        let status = tokio::process::Command::new("secret-tool")
            .args(["clear", "application", "io.github._6E6B.viewfinder"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status()
            .await
            .map_err(|_| Error::Keyring)?;
        if status.success() {
            Ok(())
        } else {
            Err(Error::Keyring)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_injected_credentials_are_rejected() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"sessionid":"synthetic","csrftoken":"synthetic","ds_user_id":"not-numeric"}),
            serde_json::json!({"sessionid":"synthetic; other=value","csrftoken":"synthetic","ds_user_id":"123"}),
            serde_json::json!({"sessionid":"synthetic","csrftoken":"synthetic\r\nInjected: value","ds_user_id":"123"}),
        ] {
            assert!(Session::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        }
    }

    #[test]
    fn accepts_synthetic_python_export() {
        let value = serde_json::json!({"sessionid":"synthetic","csrftoken":"synthetic","ds_user_id":"123","user_agent":"synthetic-agent"});
        let session = Session::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(session.values["ds_user_id"], "123");
    }
}
