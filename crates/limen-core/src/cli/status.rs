//! `lc status`: everything at one glance, for the Owner, Opus and Manager.
//! A placeholder until Manager's port fills it.

use super::*;

#[derive(clap::Args)]
pub(super) struct StatusArgs {}

pub(super) fn status_cmd(
    a: StatusArgs,
    asked: bool,
    call: impl Fn(&str, Value) -> Result<Value, RpcError>,
) -> Result<Out, RpcError> {
    let _ = (a, asked, call);
    Err(RpcError::new("unavailable", "lc status is not built yet", "Manager's port adds it"))
}
