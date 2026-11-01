pub mod interp;
pub mod lexer;
pub mod parser;

/// Run PHP source and return what it prints.
pub fn run_source(src: &str) -> Result<String, String> {
    let tokens = lexer::lex(src)?;
    let ast = parser::parse(&tokens)?;
    interp::run(&ast)
}

#[cfg(test)]
mod tests {
    use super::run_source;

    fn t(src: &str, want: &str) {
        assert_eq!(run_source(src).unwrap(), want, "src: {src}");
    }

    #[test]
    fn echo_arith() {
        t("<?php echo 1+1;", "2");
        t("<?php echo 2+3*4;", "14");
        t("<?php echo (2+3)*4;", "20");
        t("<?php echo 7/2;", "3.5");
        t("<?php echo 6/3;", "2");
        t("<?php echo -3 + 1;", "-2");
    }

    #[test]
    fn echo_strings() {
        t("<?php echo 'a' . 'b';", "ab");
        t("<?php echo \"x\\n\";", "x\n");
        t("<?php echo 'n=', 1+2;", "n=3");
        t("<?php echo 'a' . 1+2;", "a3");
    }

    #[test]
    fn inline_html() {
        t("hi <?php echo 1; ?>\nbye", "hi 1bye");
    }

    #[test]
    fn floats_and_overflow() {
        t("<?php echo 0.1+0.2;", "0.3");
        t("<?php echo 9223372036854775807 + 1;", "9.2233720368548E+18");
    }

    #[test]
    fn errors() {
        assert!(run_source("<?php echo 1/0;").is_err());
        assert!(run_source("<?php echo (1;").is_err());
    }
}
