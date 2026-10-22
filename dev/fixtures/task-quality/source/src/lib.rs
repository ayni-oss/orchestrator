//! Small dependency-free project for managed worker acceptance.
#[must_use]
pub fn greeting() -> &'static str {
    "Hello from the managed task worker"
}

#[cfg(test)]
mod tests {
    #[test]
    fn greeting_names_the_managed_task_worker() {
        assert_eq!(super::greeting(), "Hello from the managed task worker");
    }
}
