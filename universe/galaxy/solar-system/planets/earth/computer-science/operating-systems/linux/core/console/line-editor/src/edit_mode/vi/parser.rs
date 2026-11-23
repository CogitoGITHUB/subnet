use super::command::{parse_command, Command};
use super::motion::{parse_motion, Motion};
use crate::{edit_mode::vi::ViMode, EditCommand, ReedlineEvent, Vi};
use std::iter::Peekable;

#[derive(Debug, Clone)]
pub enum ReedlineOption {
    Event(ReedlineEvent),
    Edit(EditCommand),
    Incomplete,
}

impl ReedlineOption {
    pub fn into_reedline_event(self) -> Option<ReedlineEvent> {
        match self {
            ReedlineOption::Event(event) => Some(event),
            ReedlineOption::Edit(edit) => Some(ReedlineEvent::Edit(vec![edit])),
            ReedlineOption::Incomplete => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseResult<T> {
    Valid(T),
    Incomplete,
    Invalid,
}

impl<T> ParseResult<T> {
    fn is_invalid(&self) -> bool {
        match self {
            ParseResult::Valid(_) => false,
            ParseResult::Incomplete => false,
            ParseResult::Invalid => true,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParsedViSequence {
    multiplier: Option<usize>,
    command: Option<Command>,
    count: Option<usize>,
    motion: ParseResult<Motion>,
}

impl ParsedViSequence {
    pub fn is_valid(&self) -> bool {
        !self.motion.is_invalid()
    }

    pub fn is_complete(&self, mode: ViMode) -> bool {
        assert!(mode == ViMode::Normal || mode == ViMode::Visual);
        match (&self.command, &self.motion) {
            (None, ParseResult::Valid(_)) => true,
            (Some(Command::Incomplete), _) => false,
            (Some(cmd), ParseResult::Incomplete)
                if !cmd.requires_motion() || mode == ViMode::Visual =>
            {
                true
            }
            (Some(_), ParseResult::Valid(_)) => true,
            (Some(cmd), ParseResult::Incomplete) if cmd.requires_motion() => false,
            _ => false,
        }
    }

    /// Combine `multiplier` and `count` as vim only considers the product
    ///
    /// Default return value: 1
    ///
    /// ### Note:
    ///
    /// <https://github.com/vim/vim/blob/140f6d0eda7921f2f0b057ec38ed501240903fc3/runtime/doc/motion.txt#L64-L70>
    fn total_multiplier(&self) -> usize {
        self.multiplier.unwrap_or(1) * self.count.unwrap_or(1)
    }

    fn apply_multiplier(&self, raw_events: Option<Vec<ReedlineOption>>) -> ReedlineEvent {
        if let Some(raw_events) = raw_events {
            let events = std::iter::repeat_n(raw_events, self.total_multiplier())
                .flatten()
                .filter_map(ReedlineOption::into_reedline_event)
                .collect::<Vec<ReedlineEvent>>();

            if events.is_empty() || events.contains(&ReedlineEvent::None) {
                // TODO: Clarify if the `contains(ReedlineEvent::None)` path is relevant
                ReedlineEvent::None
            } else {
                ReedlineEvent::Multiple(events)
            }
        } else {
            ReedlineEvent::None
        }
    }

    pub fn changes_mode(&self, mode: ViMode) -> Option<ViMode> {
        match (&self.command, &self.motion) {
            (Some(Command::EnterViInsert), ParseResult::Incomplete)
            | (Some(Command::EnterViAppend), ParseResult::Incomplete)
            | (Some(Command::NewlineAbove), ParseResult::Incomplete)
            | (Some(Command::NewlineBelow), ParseResult::Incomplete)
            | (Some(Command::ChangeToLineEnd), ParseResult::Incomplete)
            | (Some(Command::AppendToEnd), ParseResult::Incomplete)
            | (Some(Command::PrependToStart), ParseResult::Incomplete)
            | (Some(Command::RewriteCurrentLine), ParseResult::Incomplete)
            | (Some(Command::SubstituteCharWithInsert), ParseResult::Incomplete)
            | (Some(Command::HistorySearch), ParseResult::Incomplete)
            | (Some(Command::Change), ParseResult::Valid(_)) => Some(ViMode::Insert),
            (Some(Command::Change), ParseResult::Incomplete) if mode == ViMode::Visual => {
                Some(ViMode::Insert)
            }
            (Some(Command::Delete), ParseResult::Incomplete)
            | (Some(Command::Lowercase), ParseResult::Incomplete)
            | (Some(Command::Uppercase), ParseResult::Incomplete)
            | (Some(Command::Switchcase), ParseResult::Incomplete)
                if mode == ViMode::Visual =>
            {
                Some(ViMode::Normal)
            }
            // `r<char>` in Visual replaces and returns to Normal; without this it
            // would fall through to `None` and leave the editor stuck in Visual.
            (Some(Command::ReplaceChar(_)), _) if mode == ViMode::Visual => Some(ViMode::Normal),
            (Some(Command::ChangeTextObject { .. }), _) => Some(ViMode::Insert),
            (Some(Command::Delete), ParseResult::Incomplete)
            | (Some(Command::DeleteChar), ParseResult::Incomplete)
            | (Some(Command::DeleteToEnd), ParseResult::Incomplete)
            | (Some(Command::Delete), ParseResult::Valid(_))
            | (Some(Command::DeleteChar), ParseResult::Valid(_))
            | (Some(Command::DeleteToEnd), ParseResult::Valid(_))
            | (Some(Command::Yank), ParseResult::Valid(_))
            | (Some(Command::Yank), ParseResult::Incomplete)
            | (Some(Command::DeleteTextObject { .. }), _)
            | (Some(Command::YankTextObject { .. }), _) => Some(ViMode::Normal),
            _ => None,
        }
    }

    /// Record `events` as the repeatable previous action, unless there is
    /// nothing to record or the command is itself a repeat: `.` replays the
    /// last change and must never record itself, otherwise `previous` would
    /// nest without bound. `RepeatLastAction` only reaches the no-motion arm,
    /// but guarding here keeps the recording policy correct for either caller.
    fn record_previous(vi_state: &mut Vi, command: &Command, events: &ReedlineEvent) {
        match events {
            ReedlineEvent::None => {}
            _ if matches!(command, Command::RepeatLastAction) => {}
            event => vi_state.previous = Some(event.clone()),
        }
    }

    pub fn to_reedline_event(&self, vi_state: &mut Vi) -> ReedlineEvent {
        match (&self.multiplier, &self.command, &self.count, &self.motion) {
            (_, Some(command), None, ParseResult::Incomplete) => {
                let events = self.apply_multiplier(Some(command.to_reedline(vi_state)));
                Self::record_previous(vi_state, command, &events);
                events
            }
            // This case handles all combinations of commands and motions that could exist
            (_, Some(command), _, ParseResult::Valid(motion)) => {
                let events =
                    self.apply_multiplier(command.to_reedline_with_motion(motion, vi_state));
                Self::record_previous(vi_state, command, &events);
                events
            }
            (_, None, _, ParseResult::Valid(motion)) => {
                self.apply_multiplier(Some(motion.to_reedline(vi_state)))
            }
            _ => ReedlineEvent::None,
        }
    }
}

fn parse_number<'iter, I>(input: &mut Peekable<I>) -> Option<usize>
where
    I: Iterator<Item = &'iter char>,
{
    match input.peek() {
        Some('0') => None,
        Some(x) if x.is_ascii_digit() => {
            let mut count: usize = 0;
            while let Some(&c) = input.peek() {
                if let Some(d) = c.to_digit(10) {
                    let _ = input.next();
                    count = count
                        .saturating_mul(10)
                        .saturating_add(d as usize)
                        .min(u16::MAX as usize);
                } else {
                    return Some(count);
                }
            }
            Some(count)
        }
        _ => None,
    }
}

pub fn parse<'iter, I>(mode: ViMode, input: &mut Peekable<I>) -> ParsedViSequence
where
    I: Iterator<Item = &'iter char>,
{
    let multiplier = parse_number(input);
    let command = parse_command(mode, input);
    let count = parse_number(input);
    let motion = parse_motion(input, command.as_ref().and_then(Command::whole_line_char));

    ParsedViSequence {
        multiplier,
        command,
        count,
        motion,
    }
}

