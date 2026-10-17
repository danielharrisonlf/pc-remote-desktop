//! Predefined, short input events. Delegates to the active platform's `InputBackend`.
use crate::protocol::Action;

pub fn execute(action: &Action) -> Result<(), &'static str> {
    crate::platform::input()
        .execute(action)
        .map_err(|e| e.protocol_code())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_input_actions_are_rejected_as_invalid_message() {
        assert_eq!(execute(&Action::StateGet), Err("invalid_message"));
    }
}
