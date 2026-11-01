//! Built-in classes written in PHP itself and compiled ahead of every program.

const THROWABLE_BODY: &str = r#"
    protected $message = '';
    protected $code = 0;
    protected $file = '';
    protected $line = 0;
    protected $previous = null;
    public function __construct($message = '', $code = 0, $previous = null) {
        $this->message = $message;
        $this->code = $code;
        $this->previous = $previous;
    }
    public function getMessage() { return $this->message; }
    public function getCode() { return $this->code; }
    public function getPrevious() { return $this->previous; }
    public function getFile() { return $this->file; }
    public function getLine() { return $this->line; }
    public function getTrace() { return []; }
    public function getTraceAsString() { return '#0 {main}'; }
    public function __toString() { return get_class($this) . ': ' . $this->message; }
"#;

const SIMPLE: &[(&str, &str)] = &[
    ("ErrorException", "Exception"),
    ("TypeError", "Error"),
    ("ValueError", "Error"),
    ("ArithmeticError", "Error"),
    ("DivisionByZeroError", "ArithmeticError"),
    ("AssertionError", "Error"),
    ("ArgumentCountError", "TypeError"),
    ("UnhandledMatchError", "Error"),
    ("RuntimeException", "Exception"),
    ("LogicException", "Exception"),
    ("InvalidArgumentException", "LogicException"),
    ("DomainException", "LogicException"),
    ("LengthException", "LogicException"),
    ("OutOfRangeException", "LogicException"),
    ("BadFunctionCallException", "LogicException"),
    ("BadMethodCallException", "BadFunctionCallException"),
    ("OutOfBoundsException", "RuntimeException"),
    ("RangeException", "RuntimeException"),
    ("OverflowException", "RuntimeException"),
    ("UnderflowException", "RuntimeException"),
    ("UnexpectedValueException", "RuntimeException"),
    ("JsonException", "Exception"),
    ("CompileError", "Error"),
    ("ParseError", "CompileError"),
    ("FiberError", "Error"),
];

pub fn source() -> String {
    let mut s = String::from(
        "<?php
interface Traversable {}
interface Iterator extends Traversable {}
interface IteratorAggregate extends Traversable {}
interface ArrayAccess {}
interface Countable {}
interface Stringable {}
interface JsonSerializable {}
interface UnitEnum {}
interface Throwable extends Stringable {}
class stdClass {}
class __PHP_Incomplete_Class {}
final class Closure {}
final class Generator implements Iterator {}
final class Fiber {}
interface BackedEnum extends UnitEnum {}
",
    );
    s.push_str(&format!("class Exception implements Throwable {{{THROWABLE_BODY}}}\n"));
    s.push_str(&format!("class Error implements Throwable {{{THROWABLE_BODY}}}\n"));
    for (name, parent) in SIMPLE {
        s.push_str(&format!("class {name} extends {parent} {{}}\n"));
    }
    s.push_str(include_str!("spl.php"));
    s.push_str(crate::port_hash::PRELUDE);
    s
}
