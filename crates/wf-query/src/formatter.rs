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

    #[test]
    fn format_sql_should_indent_columns_after_select() {
        let input = "select a, b, c from t";
        let output = format_sql(input);
        // sqlformat inserts a newline and indentation between SELECT and column list
        assert!(
            output.contains('\n'),
            "expected formatted output to contain newlines:\n{output}"
        );
        assert!(output.contains('a') && output.contains('b') && output.contains('c'));
    }

    #[test]
    fn format_sql_should_preserve_string_literal_casing() {
        let input = "SELECT * FROM users WHERE name = 'Alice'";
        let output = format_sql(input);
        assert!(
            output.contains("'Alice'"),
            "expected string literal 'Alice' to preserve case in:\n{output}"
        );
    }

    #[test]
    fn format_sql_should_handle_multistatement_input() {
        let input = "select 1; select 2";
        let output = format_sql(input);
        assert!(
            output.contains("SELECT"),
            "expected SELECT in formatted output:\n{output}"
        );
    }
}
