//! `lc vault`: the notes agents leave each other in the shared vault.
//! A placeholder until the vault's port fills it.

use super::*;

#[derive(Subcommand)]
pub(super) enum VaultCmd {
    /// The notes in the vault.
    Ls,
}

/// The `lc vault` verbs a worker may run: only those that look.
pub(super) fn vault_as_a_worker(action: &VaultCmd) -> Result<(), RpcError> {
    match action {
        VaultCmd::Ls => Ok(()),
    }
}

pub(super) fn vault_cmd(action: VaultCmd, asked: bool) -> Result<Option<Out>, RpcError> {
    let _ = (action, asked);
    Err(RpcError::new("unavailable", "lc vault is not built yet", "the vault's port adds it"))
}
