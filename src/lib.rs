pub mod ast;
pub mod builtins;
pub mod compiler;
pub mod fiber;
pub mod lexer;
pub mod object;
pub mod parser;
pub mod port_array;
pub mod port_hash;
pub mod port_string;
pub mod port_var;
pub mod prelude;
pub mod refbuiltins;
pub mod stdlib_arr;
pub mod stdlib_file;
pub mod stream;
pub mod stdlib_data;
pub mod stdlib_preg;
pub mod stdlib_str;
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
    execute_file(src, "")
}

pub fn execute_file(src: &str, file: &str) -> Output {
    let prelude = match parser::parse(&prelude::source()) {
        Ok(p) => p,
        Err(e) => return Output { out: String::new(), fatal: Some(format!("prelude: {e}")), exit_code: 255 },
    };
    let user = match parser::parse(src) {
        Ok(p) => p,
        Err(e) => return Output { out: String::new(), fatal: Some(format!("PHP Parse error: {e}")), exit_code: 255 },
    };
    let all: Vec<ast::Stmt> = prelude.into_iter().chain(user).collect();
    let mut prog = match compiler::compile(&all) {
        Ok(p) => p,
        Err(e) => return Output { out: String::new(), fatal: Some(format!("PHP Fatal error: {e}")), exit_code: 255 },
    };
    let mut machine = vm::Vm::new(&mut prog);
    machine.file = file.to_string();
    let (fatal, exit_code) = match machine.run_main() {
        Ok(()) => (None, 0),
        Err(vm::Flow::Exit(c)) => (None, c),
        Err(vm::Flow::Fatal(m)) => (Some(m), 255),
        Err(vm::Flow::Throw(v)) => (Some(machine.describe_uncaught(&v)), 255),
    };
    machine.flush_buffers();
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
    fn classes_and_objects() {
        t("class A { public $x = 1; function __construct($x = 5) { $this->x = $x; } function get() { return $this->x; } } $a = new A(7); echo $a->get(), $a->x;", "77");
        t("class A { function hi() { return 'A'; } } class B extends A { function hi() { return parent::hi() . 'B'; } } echo (new B)->hi();", "AB");
        t("class A { static $n = 0; const K = 3; static function inc() { return ++self::$n + static::K; } } echo A::inc(), A::inc(), A::K;", "4 5 3".replace(' ', "").as_str());
        t("interface I { const X = 9; } class A implements I {} $a = new A; echo $a instanceof I ? 'y' : 'n', A::X;", "y9");
        t("class A { public function __toString() { return 'str'; } } echo new A, 'x' . new A;", "strxstr");
        t("class P { function __construct(private int $v) {} function v() { return $this->v; } } echo (new P(4))->v();", "4");
        t("class A { public $a = []; } $o = new A; $o->a[] = 1; $o->a['k'] = 2; echo count($o->a);", "2");
        t("abstract class S { abstract function f(); static function make() { return new static; } } class T extends S { function f() { return 't'; } } echo T::make()->f();", "t");
    }

    #[test]
    fn exceptions_closures_match() {
        t("try { throw new Exception('boom'); } catch (Exception $e) { echo $e->getMessage(); } finally { echo '!'; }", "boom!");
        t("try { echo 1 % 0; } catch (DivisionByZeroError $e) { echo get_class($e); }", "DivisionByZeroError");
        t("function f() { try { return 'a'; } finally { echo 'f'; } } echo f();", "fa");
        t("class E1 extends Exception {} try { throw new E1('x', 3); } catch (RuntimeException | E1 $e) { echo $e->getCode(); }", "3");
        t("$n = 3; $f = function($x) use ($n) { return $x * $n; }; echo $f(4);", "12");
        t("$m = 2; $g = fn($x) => $x + $m; echo $g(5), implode(',', array_map(fn($v) => $v * 2, [1, 2, 3]));", "71,2,3".replace("71,2,3", "72,4,6").as_str());
        t("echo match(2) { 1 => 'a', 2, 3 => 'b', default => 'c' };", "b");
        t("try { echo match(9) { 1 => 'a' }; } catch (\\UnhandledMatchError $e) { echo $e->getMessage(); }", "Unhandled match case 9");
    }

    #[test]
    fn references() {
        t("function inc(&$x) { $x++; } $a = 1; inc($a); inc($a); echo $a;", "3");
        t("$a = [1,2,3]; foreach ($a as &$v) { $v *= 2; } unset($v); echo implode(',', $a);", "2,4,6");
        t("$a = 1; $b = &$a; $b = 5; echo $a;", "5");
        t("function f() { static $n = 0; return ++$n; } f(); f(); echo f();", "3");
        t("$g = 10; function h() { global $g; $g++; } h(); echo $g;", "11");
        t("$a = [3,1,2]; sort($a); echo implode(',', $a); usort($a, fn($x, $y) => $y <=> $x); echo implode(',', $a);", "1,2,33,2,1");
        t("function s(...$xs) { return array_sum($xs); } echo s(1,2,3), s(...[4,5]);", "69");
        t("$a = []; array_push($a, 1, 2); array_pop($a); echo count($a);", "1");
        t("$c = 0; $f = function() use (&$c) { $c++; }; $f(); $f(); echo $c;", "2");
        t("$a = ['x' => [3,1]]; sort($a['x']); echo implode(',', $a['x']);", "1,3");
        t("class S { public $l = [2,1]; function go() { sort($this->l); return implode(',', $this->l); } } echo (new S)->go();", "1,2");
    }

    #[test]
    fn destructuring_generators_enums() {
        t("[$a, $b] = [1, 2]; echo $a, $b; ['x' => $x, 'y' => $y] = ['x' => 3, 'y' => 4]; echo $x, $y; list(, $c) = [5, 6]; echo $c;", "12346");
        t("foreach ([[1, 2], [3, 4]] as [$p, $q]) { echo $p + $q, ','; }", "3,7,");
        t("[[$a, $b], $c] = [[1, 2], 3]; echo $a, $b, $c;", "123");
        t("function g() { yield 1; yield 2; return 3; } $g = g(); foreach ($g as $v) { echo $v; } echo $g->getReturn();", "123");
        t("function g() { yield 'a' => 1; yield 'b' => 2; } foreach (g() as $k => $v) { echo $k, $v; }", "a1b2");
        t("function e() { $x = yield 1; echo 'got ', $x; $y = yield 2; echo $y; } $g = e(); echo $g->current(); echo $g->send('A'); $g->send('B');", "1got A2B");
        t("function i() { yield 1; yield from [2, 3]; yield 4; } echo implode(',', iterator_to_array(i(), false));", "1,2,3,4");
        t("function inner() { yield 1; return 'r'; } function outer() { $r = yield from inner(); echo $r; } foreach (outer() as $v) { echo $v; }", "1r");
        t("enum S: string { case A = 'a'; case B = 'b'; public function label() { return ucfirst($this->value); } } echo S::A->value, S::from('b')->name, count(S::cases()), S::A->label(); var_dump(S::tryFrom('z'));", "aB2ANULL\n");
        t("enum Suit { case H; case S; } var_dump(Suit::H === Suit::H, Suit::H === Suit::S); var_dump(Suit::H instanceof UnitEnum);", "bool(true)\nbool(false)\nbool(true)\n".replace("\\n", "\n").as_str());
        t("$o = new class(3) { function __construct(public $n) {} function d() { return $this->n * 2; } }; echo $o->d();", "6");
        t("$o = (object)['a' => 1]; echo $o->a; $o->b = 2; echo count((array)$o);", "12");
    }

    #[test]
    fn types() {
        t("function f(int $x): string { return $x; } var_dump(f('5'));", "string(1) \"5\"\n");
        t("function f(int $x) {} try { f('abc'); } catch (TypeError $e) { echo get_class($e); }", "TypeError");
        t("function g(): int { return 'x'; } try { g(); } catch (TypeError $e) { echo $e->getMessage(); }", "g(): Return value must be of type int, string returned");
        t("function h(?int $x = null, int|string $y = 2) { return $x ?? $y; } echo h(), h(7), h(null, 'z');", "27z");
        t("class A { public int $n; } $a = new A; try { echo $a->n; } catch (Error $e) { echo $e->getMessage(); }", "Typed property A::$n must not be accessed before initialization");
        t("class P { public readonly int $x; function __construct() { $this->x = 1; } } $p = new P; try { $p->x = 2; } catch (Error $e) { echo $e->getMessage(); }", "Cannot modify readonly property P::$x");
        t("class T { public int $v = 0; } $t = new T; $t->v = '12'; var_dump($t->v); try { $t->v = 'x'; } catch (TypeError $e) { echo $e->getMessage(); }", "int(12)\nCannot assign string to property T::$v of type int");
    }

    #[test]
    fn named_args_and_variable_variables() {
        t("function f($a, $b = 2, $c = 3) { return \"$a-$b-$c\"; } echo f(1, c: 9), f(b: 5, a: 0);", "1-2-90-5-3");
        t("class P { function __construct(public $x = 1, public $y = 2) {} } $p = new P(y: 7); echo $p->x, $p->y;", "17");
        t("$n = 'v'; $$n = 5; echo $v, $$n; ${'w' . 'x'} = 2; echo $wx;", "552");
        t("$a = 1; $b = 2; $r = compact('a', 'b'); echo count($r); extract(['q' => 8]); echo $q;", "28");
        t("function g() { $z = 4; return get_defined_vars(); } echo g()['z'];", "4");
    }

    #[test]
    fn fibers() {
        t("$f = new Fiber(function($x) { $y = Fiber::suspend($x * 2); echo 'got ', $y, ';'; return 'done'; }); echo $f->start(5), ';'; $f->resume('r'); echo $f->getReturn(), $f->isTerminated() ? 'T' : 'F';", "10;got r;doneT");
        t("function deep($n) { if ($n == 0) { return Fiber::suspend('bottom'); } return deep($n - 1); } $f = new Fiber(fn() => deep(5)); echo $f->start(); $f->resume(1); echo $f->isTerminated() ? 'T' : 'F';", "bottomT");
        t("$f = new Fiber(function() { try { Fiber::suspend(1); } catch (Exception $e) { echo 'caught ', $e->getMessage(); } }); $f->start(); $f->throw(new Exception('x'));", "caught x");
    }

    #[test]
    fn stdlib() {
        t("echo json_encode(['a' => 1, 'b' => [1, 2], 'c' => null, 'd' => 1.5, 'e' => 'x/y']);", "{\"a\":1,\"b\":[1,2],\"c\":null,\"d\":1.5,\"e\":\"x\\/y\"}");
        t("var_dump(json_decode('{\"a\":[1,{\"b\":2}]}', true)['a'][1]['b']);", "int(2)\n");
        t("echo json_decode('{\"x\":5}')->x, json_last_error();", "50");
        t("echo serialize(['a' => 1, 2 => 'z']);", "a:2:{s:1:\"a\";i:1;i:2;s:1:\"z\";}");
        t("class P { public $a = 1; protected $b = [2]; } $s = serialize(new P); $o = unserialize($s); echo get_class($o), count($o->b ?? []);", "P1");
        t("echo md5('a'), ' ', crc32('hello'), ' ', base64_encode('hello'), ' ', strtoupper(bin2hex('ab'));", "0cc175b9c0f1b6a831c399e269772661 907060870 aGVsbG8= 6162");
        t("echo date('Y-m-d H:i:s', 86400 * 365), ' ', ucwords('hello big world'), ' ', str_pad('5', 3, '0', STR_PAD_LEFT);", "1971-01-01 00:00:00 Hello Big World 005");
        t("echo implode(',', array_diff([1, 2, 3, 4], [2, 4])), '|', implode(',', array_column([['id' => 1], ['id' => 2]], 'id')), '|', levenshtein('kitten', 'sitting');", "1,3|1,2|3");
    }

    #[test]
    fn preg_and_buffers() {
        t("echo preg_match('/(\\d+)-(\\d+)/', 'ab 12-34 cd', $m), $m[1], $m[2];", "11234");
        t("echo preg_replace('/\\s+/', ' ', \"a  b\\n c\"), preg_replace('/(a)(b)?/', '[$1|$2]', 'ac');", "a b c[a|]c");
        t("echo implode(',', preg_split('/[\\s,]+/', 'a, b  c,d'));", "a,b,c,d");
        t("echo preg_match_all('/a(b)?/', 'ab a ab', $m), count($m), implode('', $m[0]);", "32aba ab".replace(' ', "").as_str());
        t("echo preg_replace_callback('/\\d+/', fn($m) => $m[0] * 2, 'a1 b22');", "a2 b44");
        t("echo preg_match('/(?<y>\\d{4})/', 'in 2024', $m), $m['y'];", "12024");
        t("ob_start(); echo 'hidden'; $x = ob_get_clean(); echo strtoupper($x);", "HIDDEN");
    }

    #[test]
    fn files() {
        t("$p = tempnam('/tmp', 'ph'); file_put_contents($p, \"a\\nb\\nc\\n\"); $h = fopen($p, 'r'); $n = 0; while (($l = fgets($h)) !== false) { $n++; } fclose($h); echo $n, count(file($p)), filesize($p); unlink($p); var_dump(file_exists($p));", "336bool(false)\n");
        t("$h = fopen('php://memory', 'w+'); fwrite($h, 'hello'); rewind($h); echo fread($h, 3), ftell($h), stream_get_contents($h); var_dump(is_resource($h));", "hel3lobool(true)\n");
        t("echo basename('/a/b/c.txt'), dirname('/a/b/c.txt'), pathinfo('x/y.tar.gz', PATHINFO_EXTENSION);", "c.txt/a/bgz");
        t("echo json_encode(parse_url('https://u:p@h.com:81/p/q?x=1#f'));", "{\"scheme\":\"https\",\"host\":\"h.com\",\"port\":81,\"user\":\"u\",\"pass\":\"p\",\"path\":\"\\/p\\/q\",\"query\":\"x=1\",\"fragment\":\"f\"}");
        t("fwrite(STDOUT, 'direct'); echo gettype(STDERR);", "directresource");
    }

    #[test]
    fn inline_html_and_errors() {
        assert_eq!(run_source("a<?php echo 1; ?>\nb").unwrap(), "a1b");
        assert!(run_source("<?php echo 1/0;").is_err());
        assert!(run_source("<?php nope();").is_err());
    }
}
