//! `lc mcp`: tasks, workers and the vault as MCP tools over stdio.
//! A placeholder until the MCP port fills it.

use super::*;

#[derive(clap::Args)]
pub(super) struct McpArgs {}

pub(super) fn mcp_cmd(a: McpArgs, paths: &Paths) -> Result<(), RpcError> {
    let _ = (a, paths);
    Err(RpcError::new("unavailable", "lc mcp is not built yet", "the MCP port adds it"))
}
