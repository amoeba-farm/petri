//! The signing configuration futures and oracle commands hand to
//! `wallet_signer`: which keypair or hardware wallet signs, and whether a
//! group- or world-readable keypair file is accepted.

#[derive(Clone, Debug)]
pub struct OnchainConfig {
    pub keypair_path: Option<String>,
    pub allow_insecure_keypair: bool,
}
