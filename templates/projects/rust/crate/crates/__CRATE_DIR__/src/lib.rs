pub fn project_name() -> &'static str {
    "__REPO_NAME__"
}

#[cfg(test)]
mod tests {
    use super::project_name;

    #[test]
    fn returns_repo_name() {
        let expected = "__REPO_NAME__";
        assert_eq!(project_name(), expected);
    }
}
