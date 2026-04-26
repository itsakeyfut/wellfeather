use sqlformat::{FormatOptions, QueryParams, format};

pub fn format_sql(sql: &str) -> String {
    let opts = FormatOptions {
        uppercase: Some(true),
        ..FormatOptions::default()
    };
    format(sql, &QueryParams::None, &opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_sql_should_uppercase_keywords_and_indent() {
        let input = "select * from users where id = 1";
        let output = format_sql(input);
        assert!(
            output.contains("SELECT"),
            "expected SELECT uppercase in:\n{output}"
        );
        assert!(
            output.contains("FROM"),
            "expected FROM uppercase in:\n{output}"
        );
        assert!(
            output.contains("WHERE"),
            "expected WHERE uppercase in:\n{output}"
        );
    }

    #[test]
    fn format_sql_should_return_empty_for_empty_input() {
        assert_eq!(format_sql(""), "");
    }

    #[test]
    fn format_sql_should_handle_already_uppercase() {
        let input = "SELECT id FROM users";
        let output = format_sql(input);
        assert!(output.contains("SELECT"));
        assert!(output.contains("FROM"));
    }
}
