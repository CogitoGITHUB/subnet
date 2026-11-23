use super::{Context, Module};
use crate::modules::cc::{Lang, module as cc_module};

/// Creates a module with the current C compiler and version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    cc_module(context, Lang::C)
}

