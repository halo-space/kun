use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateInfo {
    pub subject: Option<String>,
    pub issuer: Option<String>,
}
