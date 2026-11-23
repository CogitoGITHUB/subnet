use {
    crate::{
        pattern::*,
        verb::VerbInvocation,
    },
    bet::BeTree,
    std::fmt,
};

/// An intermediate parsed representation of the raw string
#[derive(Debug, Clone, PartialEq)]
pub struct CommandParts {
    pub raw_pattern: String, // may be empty
    pub pattern: BeTree<PatternOperator, PatternParts>,
    pub verb_invocation: Option<VerbInvocation>, // may be empty if user typed the separator but no char after
}

impl fmt::Display for CommandParts {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        write!(f, "{}", self.raw_pattern)?;
        if let Some(invocation) = &self.verb_invocation {
            write!(f, "{invocation}")?;
        }
        Ok(())
    }
}

impl CommandParts {
    pub fn has_not_empty_verb_invocation(&self) -> bool {
        self.verb_invocation
            .as_ref()
            .is_some_and(|vi| !vi.is_empty())
    }
    pub fn from<S: Into<String>>(raw: S) -> Self {
        let mut raw = raw.into();
        let mut invocation_start_pos: Option<usize> = None;
        let mut pt = BeTree::new();
        let mut chars = raw.char_indices().peekable();
        let mut escape_cur_char = false;
        let mut escape_next_char = false;
        // we loop on chars and build the pattern tree until we reach an unescaped ' ' or ':'
        while let Some((pos, cur_char)) = chars.next() {
            let between_slashes = pt
                .current_atom()
                .is_some_and(|pp: &PatternParts| pp.is_between_slashes());
            match cur_char {
                c if escape_cur_char => {
                    // Escaping is used to prevent characters from being consumed at the
                    // composite pattern level (or, and, parens) or as the separator between
                    // the pattern and the verb. An escaped char is usable in a pattern atom.
                    pt.mutate_or_create_atom(PatternParts::default).push(c);
                }
                '\\' => {
                    // Pattern escaping rules:
                    // 	- when after '/': only ' ', ':',  '/' and '\' need escaping
                    // 	- otherwise, '&,' '|', '(', ')' need escaping too ('(' is only here for
                    // 	symmetry)
                    let between_slashes = match pt.current_atom() {
                        Some(pattern_parts) => pattern_parts.is_between_slashes(),
                        None => false,
                    };
                    escape_next_char = match chars.peek() {
                        None => false, // End of the string, we can't be escaping
                        Some((_, next_char)) => match (next_char, between_slashes) {
                            (' ' | ':' | '/' | '\\', _) => true,
                            ('&' | '|' | '!' | '(' | ')', false) => true,
                            _ => false,
                        },
                    };
                    if !escape_next_char {
                        // if the '\' isn't used for escaping, it's used as its char value
                        pt.mutate_or_create_atom(PatternParts::default).push('\\');
                    }
                }
                ':' => {
                    if matches!(chars.peek(), Some((_, ':'))) {
                        // two successive ':' in pattern position are part of the
                        // pattern
                        pt.mutate_or_create_atom(PatternParts::default).push(':');
                        escape_next_char = true;
                    } else {
                        // ending the pattern part
                        invocation_start_pos = Some(pos);
                        break;
                    }
                }
                ' ' => {
                    // ending the pattern part
                    invocation_start_pos = Some(pos);
                    break;
                }
                '/' => {
                    // starting an atom part
                    pt.mutate_or_create_atom(PatternParts::default).add_part();
                }
                '|' if !between_slashes && pt.accept_binary_operator() => {
                    pt.push_operator(PatternOperator::Or);
                }
                '&' if !between_slashes && pt.accept_binary_operator() => {
                    pt.push_operator(PatternOperator::And);
                }
                '!' if !between_slashes && pt.accept_unary_operator() => {
                    pt.push_operator(PatternOperator::Not);
                }
                '(' if !between_slashes && pt.accept_opening_par() => {
                    pt.open_par();
                }
                ')' if !between_slashes && pt.accept_closing_par() => {
                    pt.close_par();
                }
                _ => {
                    pt.mutate_or_create_atom(PatternParts::default)
                        .push(cur_char);
                }
            }
            escape_cur_char = escape_next_char;
            escape_next_char = false;
        }
        let mut verb_invocation = None;
        if let Some(pos) = invocation_start_pos {
            verb_invocation = Some(VerbInvocation::from(
                raw[pos + 1..].trim_start(), // allowing extra spaces
            ));
            raw.truncate(pos);
        }
        CommandParts {
            raw_pattern: raw,
            pattern: pt,
            verb_invocation,
        }
    }

    /// split an input into its two possible parts, the pattern
    /// and the verb invocation. Each part, when defined, is
    /// suitable to create a command on its own.
    pub fn split(mut self) -> (Option<CommandParts>, Option<CommandParts>) {
        let verb_invocation = self.verb_invocation.take();
        (
            if self.raw_pattern.is_empty() {
                None
            } else {
                Some(CommandParts {
                    raw_pattern: self.raw_pattern,
                    pattern: self.pattern,
                    verb_invocation: None,
                })
            },
            verb_invocation.map(|verb_invocation| CommandParts {
                raw_pattern: String::new(),
                pattern: BeTree::new(),
                verb_invocation: Some(verb_invocation),
            }),
        )
    }
}

