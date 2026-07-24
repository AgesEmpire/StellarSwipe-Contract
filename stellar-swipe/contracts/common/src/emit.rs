/// Emit a contract event with a single-Symbol topic tuple.
///
/// Standardizes the pattern of constructing a 1-element topic tuple and
/// calling `env.events().publish(...)` at every event call site.
///
/// # Usage
///
/// ```ignore
/// // Instead of:
/// let topics = (soroban_sdk::Symbol::new(env, "my_event"),);
/// env.events().publish(topics, (field1, field2));
///
/// // Write:
/// stellar_swipe_common::emit_event!(env, "my_event", (field1, field2));
/// ```
///
/// The macro expands to a single `env.events().publish(...)` call with a
/// 1-element topic tuple containing the given event name as a `Symbol`.
///
/// See `docs/emit_event_macro.md` for the full usage guide.
#[macro_export]
macro_rules! emit_event {
    ($env:expr, $name:literal, $data:expr) => {
        $env.events()
            .publish((soroban_sdk::Symbol::new($env, $name),), $data)
    };
}
