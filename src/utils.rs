/// This is a stable alternative to rust's unstable feature [str_split_once](https://github.com/rust-lang/rust/issues/74773).
pub fn split_once(string: &str, pattern: char) -> Option<(&str, &str)> {
    string.split_once(pattern)
}

pub fn unwrap_result<T>(result: Result<T, T>) -> T {
    match result {
        Ok(t) => t,
        Err(t) => t,
    }
}
