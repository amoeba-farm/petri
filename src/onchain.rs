use crate::backend::CliError;
#[derive(Clone, Debug)]
pub struct OnchainConfig {
    pub network: String,
    pub backend_url: String,
    pub commitment: Option<String>,
    pub keypair_path: Option<String>,
    pub allow_insecure_keypair: bool,
}

pub fn resolve_rpc_url(config: &OnchainConfig) -> Result<String, CliError> {
    crate::petri_config::rpc_gateway_url(&config.backend_url).map_err(CliError::new)
}
