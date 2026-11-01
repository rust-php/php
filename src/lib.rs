pub mod ast;
pub mod builtins;
pub mod compiler;
pub mod lexer;
pub mod parser;
pub mod value;
pub mod vm;

pub struct Output {
    pub out: String,
    /// Uncaught fatal error message, if any.
    pub fatal: Option<String>,
    pub exit_code: i32,
}

/// Compile and run PHP source, returning everything it printed.
pub fn execute(src: &str) -> Output {
    let prog = match parser::parse(src).and_then(|ast| compiler::compile(&ast)) {
        Ok(p) => p,
        Err(e) => return Output { out: String::new(), fatal: Some(format!("PHP Parse error: {e}")), exit_code: 255 },
    };
    let mut machine = vm::Vm::new(&prog);
    let (fatal, exit_code) = match machine.run_main() {
        Ok(()) => (None, 0),
        Err(vm::Flow::Exit(c)) => (None, c),
        Err(vm::Flow::Fatal(m)) => (Some(m), 255),
    };
    Output { out: std::mem::take(&mut machine.out), fatal, exit_code }
}

/// Run PHP source and return its output, or the fatal error message.
pub fn run_source(src: &str) -> Result<String, String> {
    let o = execute(src);
    match o.fatal {
        Some(m) => Err(m),
        None => Ok(o.out),
    }
}

#[cfg(test)]
mod tests {
    use super::run_source;

    fn t(src: &str, want: &str) {
        let full = format!("<?php {src}");
        assert_eq!(run_source(&full).unwrap_or_else(|e| format!("ERR: {e}")), want, "src: {src}");
    }

    #[test]
    fn arithmetic_and_strings() {
        t("echo 1+1;", "2");
        t("echo 2+3*4, (2+3)*4;", "1420");
        t("echo 7/2, 6/3, 7%3, 2**10, 2**-1;", "3.52110240.5");
    }

    #[test]
    fn variables_and_control_flow() {
        t("$a = 5; $a += 2; echo $a;", "7");
        t("$i = 0; while ($i < 3) { echo $i; $i++; }", "012");
        t("for ($i = 0; $i < 5; $i++) { if ($i == 3) continue; if ($i == 4) break; echo $i; }", "012");
        t("$i = 0; do { echo $i; } while (++$i < 3);", "012");
        t("switch (2) { case 1: echo 'a'; break; case 2: echo 'b'; case 3: echo 'c'; break; default: echo 'd'; }", "bc");
        t("echo 1 > 0 ? 'y' : 'n', null ?? 'd', 0 ?: 'e';", "yde");
    }

    #[test]
    fn functions() {
        t("function fib($n) { return $n < 2 ? $n : fib($n-1) + fib($n-2); } echo fib(20);", "6765");
        t("function g($a, $b = 10) { return $a + $b; } echo g(1), g(1, 2);", "113");
    }

    #[test]
    fn arrays() {
        t("$a = [1, 2, 3]; $a[] = 4; echo count($a), $a[3];", "44");
        t("$a = ['x' => 1]; $a['y']['z'] = 2; echo $a['y']['z'];", "2");
        t("$s = 0; foreach ([1,2,3] as $k => $v) { $s += $k * $v; } echo $s;", "8");
        t("$a = [1,2]; $b = $a; $b[] = 3; echo count($a), count($b);", "23");
        t("$a = [1,2,3]; unset($a[1]); echo implode(',', $a);", "1,3");
        t("echo isset($x), isset($y['k']) ? 'y' : 'n';", "n");
    }

    #[test]
    fn strings_and_interpolation() {
        t("$n = 'w'; echo \"hi $n! {$n}x\\n\";", "hi w! wx\n");
        t("$a = ['k' => 'v']; echo \"$a[k]\";", "v");
        t("echo 'a' . 1 + 2;", "a3");
        t("echo sprintf('%05.1f|%-4s|%3d|%x', 3.14159, 'ab', 7, 255);", "003.1|ab  |  7|ff");
        t("echo strlen('héllo'), strtoupper('abc'), str_repeat('ab', 2);", "6ABCabab");
    }

    #[test]
    fn dumps() {
        t("var_dump(1, 1.5, 'a', true, null);", "int(1)\nfloat(1.5)\nstring(1) \"a\"\nbool(true)\nNULL\n");
        t("print_r([1, [2]]);", "Array\n(\n    [0] => 1\n    [1] => Array\n        (\n            [0] => 2\n        )\n\n)\n");
        t("echo 0.1 + 0.2, ' ', 1e100, ' ', 9223372036854775807 + 1;", "0.3 1.0E+100 9.2233720368548E+18");
    }

    #[test]
    fn inline_html_and_errors() {
        assert_eq!(run_source("a<?php echo 1; ?>\nb").unwrap(), "a1b");
        assert!(run_source("<?php echo 1/0;").is_err());
        assert!(run_source("<?php nope();").is_err());
    }
}
