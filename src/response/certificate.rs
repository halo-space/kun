#[derive(Debug, Clone, Default)]
pub struct CertificateInfo {
    pub subject: Option<String>,
    pub issuer: Option<String>,
}
