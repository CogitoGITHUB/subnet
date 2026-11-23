use std::hash::Hash;

use crate::{
    action::{Action, Actions},
    config::{Keybindings, merge_keybindings},
    event::Key,
    television::Mode,
    utils::hashmaps::invert_hashmap,
};
use rustc_hash::FxHashMap;

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct InputMap {
    pub global_keybindings: Keybindings,
    pub channel_keybindings: Keybindings,

    /// This is a reverse mapping of global and channel keybindings
    /// to facilitate lookups from actions to keys.
    actions_keys: FxHashMap<Actions, Key>,
}

impl Hash for InputMap {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        for (a, k) in &self.actions_keys {
            (a, k).hash(state);
        }
    }
}

impl InputMap {
    pub fn new(global: Keybindings, channel: Keybindings) -> Self {
        let merged = merge_keybindings(global.clone(), &channel);
        let actions_keys = invert_hashmap(&merged);
        Self {
            global_keybindings: global,
            channel_keybindings: channel,
            actions_keys,
        }
    }

    /// Gets all actions bound to a specific key for the current mode.
    ///
    /// - `Mode::Channel` checks both global and channel-specific keybindings.
    /// - `Mode::RemoteControl` and `Mode::ActionPicker` only check global keybindings.
    pub fn get_actions_for_key(
        &self,
        key: &Key,
        mode: &Mode,
    ) -> Option<&Actions> {
        match mode {
            Mode::RemoteControl | Mode::ActionPicker => {
                self.global_keybindings.get(key)
            }
            Mode::Channel => self
                .channel_keybindings
                .get(key)
                .or_else(|| self.global_keybindings.get(key)),
        }
    }

    /// Gets the key associated with a specific action.
    pub fn get_key_for_action(&self, action: &Action) -> Option<Key> {
        self.actions_keys
            .get(&Actions::single(action.clone()))
            .copied()
    }

    /// Merges another set of global keybindings into the existing ones.
    ///
    /// This won't override channel specific keybindings.
    pub fn merge_globals_with(&mut self, keybindings: &Keybindings) {
        for (key, action) in keybindings.iter() {
            self.global_keybindings.insert(*key, action.clone());
        }
        // Update actions_keys to reflect the merged actions
        // but only if they aren't already mapped in channel specific keybindings
        for (key, actions) in keybindings.iter() {
            if self.channel_keybindings.contains_key(key) {
                continue;
            }
            self.actions_keys.insert(actions.clone(), *key);
        }
    }
}

