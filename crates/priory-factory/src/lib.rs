pub mod case;
pub mod draw;
pub mod explain;
pub mod factory;
pub mod guard;
pub mod labels;
pub mod lanes;
pub mod lines;
pub mod paths;
pub mod readiness;
pub mod rules;

/// A count and its noun, such as `1 task` or `2 tasks`; `many` covers irregular plurals.
pub fn plural(count: usize, one: &str, many: &str) -> String {
    match count {
        1 => format!("1 {one}"),
        count => format!("{count} {many}"),
    }
}
