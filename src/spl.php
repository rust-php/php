interface OuterIterator extends Iterator {}
interface RecursiveIterator extends Iterator {}
interface SeekableIterator extends Iterator {}
interface SplObserver {}
interface SplSubject {}
interface Serializable {}

class EmptyIterator implements Iterator {
    public function current(): mixed { throw new BadMethodCallException('Accessing the value of an EmptyIterator'); }
    public function key(): mixed { throw new BadMethodCallException('Accessing the key of an EmptyIterator'); }
    public function next(): void {}
    public function rewind(): void {}
    public function valid(): bool { return false; }
}

class ArrayIterator implements SeekableIterator, ArrayAccess, Countable {
    private $__a;
    private $__k = [];
    private $__i = 0;
    public function __construct($array = []) { $this->__a = $array; $this->__k = array_keys($array); }
    public function current(): mixed { return $this->valid() ? $this->__a[$this->__k[$this->__i]] : null; }
    public function key(): mixed { return $this->__k[$this->__i] ?? null; }
    public function next(): void { $this->__i++; }
    public function rewind(): void { $this->__i = 0; $this->__k = array_keys($this->__a); }
    public function valid(): bool { return $this->__i < count($this->__k); }
    public function seek($pos) { $this->__i = $pos; }
    public function count(): int { return count($this->__a); }
    public function offsetExists($o): bool { return isset($this->__a[$o]); }
    public function offsetGet($o): mixed { return $this->__a[$o]; }
    public function offsetSet($o, $v): void { if ($o === null) { $this->__a[] = $v; } else { $this->__a[$o] = $v; } }
    public function offsetUnset($o): void { unset($this->__a[$o]); }
    public function getArrayCopy() { return $this->__a; }
}

class ArrayObject implements IteratorAggregate, ArrayAccess, Countable {
    private $__a;
    public function __construct($array = []) { $this->__a = $array; }
    public function getIterator(): Iterator { return new ArrayIterator($this->__a); }
    public function count(): int { return count($this->__a); }
    public function offsetExists($o): bool { return isset($this->__a[$o]); }
    public function offsetGet($o): mixed { return $this->__a[$o]; }
    public function offsetSet($o, $v): void { if ($o === null) { $this->__a[] = $v; } else { $this->__a[$o] = $v; } }
    public function offsetUnset($o): void { unset($this->__a[$o]); }
    public function append($v) { $this->__a[] = $v; }
    public function getArrayCopy() { return $this->__a; }
}

class IteratorIterator implements OuterIterator {
    protected $__it;
    public function __construct($iterator) {
        while ($iterator instanceof IteratorAggregate) { $iterator = $iterator->getIterator(); }
        $this->__it = $iterator;
    }
    public function getInnerIterator() { return $this->__it; }
    public function current(): mixed { return $this->__it->current(); }
    public function key(): mixed { return $this->__it->key(); }
    public function next(): void { $this->__it->next(); }
    public function rewind(): void { $this->__it->rewind(); }
    public function valid(): bool { return $this->__it->valid(); }
}

class LimitIterator extends IteratorIterator {
    private $__off; private $__lim; private $__pos = 0;
    public function __construct($iterator, $offset = 0, $limit = -1) { parent::__construct($iterator); $this->__off = $offset; $this->__lim = $limit; }
    public function rewind(): void { $this->__it->rewind(); $this->__pos = 0; while ($this->__pos < $this->__off && $this->__it->valid()) { $this->__it->next(); $this->__pos++; } }
    public function valid(): bool { return ($this->__lim == -1 || $this->__pos < $this->__off + $this->__lim) && $this->__it->valid(); }
    public function next(): void { $this->__pos++; $this->__it->next(); }
}

class CallbackFilterIterator extends IteratorIterator {
    private $__cb;
    public function __construct($iterator, $callback) { parent::__construct($iterator); $this->__cb = $callback; }
    private function __skip() { while ($this->__it->valid() && !call_user_func($this->__cb, $this->__it->current(), $this->__it->key(), $this->__it)) { $this->__it->next(); } }
    public function rewind(): void { $this->__it->rewind(); $this->__skip(); }
    public function next(): void { $this->__it->next(); $this->__skip(); }
}

class SplDoublyLinkedList implements Iterator, Countable, ArrayAccess {
    const IT_MODE_LIFO = 2;
    const IT_MODE_FIFO = 0;
    const IT_MODE_DELETE = 1;
    const IT_MODE_KEEP = 0;
    protected $__d = [];
    protected $__mode = 0;
    private $__i = 0;
    public function push($v) { $this->__d[] = $v; }
    public function pop() { if (!$this->__d) throw new RuntimeException("Can't pop from an empty datastructure"); return array_pop($this->__d); }
    public function shift() { if (!$this->__d) throw new RuntimeException("Can't shift from an empty datastructure"); return array_shift($this->__d); }
    public function unshift($v) { array_unshift($this->__d, $v); }
    public function top() { if (!$this->__d) throw new RuntimeException("Can't peek at an empty datastructure"); return $this->__d[count($this->__d) - 1]; }
    public function bottom() { if (!$this->__d) throw new RuntimeException("Can't peek at an empty datastructure"); return $this->__d[0]; }
    public function isEmpty() { return !$this->__d; }
    public function count(): int { return count($this->__d); }
    public function toArray() { return $this->__d; }
    public function setIteratorMode($m) { $this->__mode = $m; return $m; }
    public function offsetExists($o): bool { return $o >= 0 && $o < count($this->__d); }
    public function offsetGet($o): mixed { if ($o < 0 || $o >= count($this->__d)) throw new OutOfRangeException('SplDoublyLinkedList::offsetGet(): Argument #1 ($index) is out of range'); return $this->__d[$o]; }
    public function offsetSet($o, $v): void { if ($o === null) { $this->__d[] = $v; } else { $this->__d[$o] = $v; } }
    public function offsetUnset($o): void { unset($this->__d[$o]); $this->__d = array_values($this->__d); }
    public function rewind(): void { $this->__i = ($this->__mode & 2) ? count($this->__d) - 1 : 0; }
    public function valid(): bool { return $this->__i >= 0 && $this->__i < count($this->__d); }
    public function current(): mixed { return $this->__d[$this->__i] ?? null; }
    public function key(): mixed { return $this->__i; }
    public function next(): void { if ($this->__mode & 2) { $this->__i--; } else { $this->__i++; } }
}
class SplQueue extends SplDoublyLinkedList {
    public function enqueue($v) { $this->push($v); }
    public function dequeue() { return $this->shift(); }
}
class SplStack extends SplDoublyLinkedList {
    protected $__mode = 2;
}

class SplObjectStorage implements Countable, Iterator, ArrayAccess {
    private $__o = [];
    private $__dt = [];
    private $__i = 0;
    public function attach($obj, $inf = null) { $h = spl_object_id($obj); $this->__o[$h] = $obj; $this->__dt[$h] = $inf; }
    public function detach($obj) { $h = spl_object_id($obj); unset($this->__o[$h]); unset($this->__dt[$h]); }
    public function contains($obj) { return isset($this->__o[spl_object_id($obj)]); }
    public function count(): int { return count($this->__o); }
    public function offsetExists($obj): bool { return $this->contains($obj); }
    public function offsetGet($obj): mixed { $h = spl_object_id($obj); if (!isset($this->__o[$h])) throw new UnexpectedValueException('Object not found'); return $this->__dt[$h]; }
    public function offsetSet($obj, $inf): void { $this->attach($obj, $inf); }
    public function offsetUnset($obj): void { $this->detach($obj); }
    public function getInfo() { $k = array_keys($this->__o); return $this->__dt[$k[$this->__i]] ?? null; }
    public function setInfo($inf) { $k = array_keys($this->__o); if (isset($k[$this->__i])) $this->__dt[$k[$this->__i]] = $inf; }
    public function rewind(): void { $this->__i = 0; }
    public function valid(): bool { return $this->__i < count($this->__o); }
    public function key(): mixed { return $this->__i; }
    public function current(): mixed { $k = array_keys($this->__o); return $this->__o[$k[$this->__i]] ?? null; }
    public function next(): void { $this->__i++; }
}

class SplFixedArray implements ArrayAccess, Countable, IteratorAggregate {
    private $__d = [];
    private $__n = 0;
    public function __construct($size = 0) { $this->__n = $size; $this->__d = $size > 0 ? array_fill(0, $size, null) : []; }
    public function getSize() { return $this->__n; }
    public function setSize($n) { for ($i = $this->__n; $i < $n; $i++) $this->__d[$i] = null; for ($i = $n; $i < $this->__n; $i++) unset($this->__d[$i]); $this->__n = $n; }
    public function count(): int { return $this->__n; }
    public function toArray() { return $this->__d; }
    public function offsetExists($o): bool { return is_int($o) && $o >= 0 && $o < $this->__n && $this->__d[$o] !== null; }
    public function offsetGet($o): mixed { if (!is_numeric($o) || $o < 0 || $o >= $this->__n) throw new RuntimeException('Index invalid or out of range'); return $this->__d[(int)$o]; }
    public function offsetSet($o, $v): void { if (!is_numeric($o) || $o < 0 || $o >= $this->__n) throw new RuntimeException('Index invalid or out of range'); $this->__d[(int)$o] = $v; }
    public function offsetUnset($o): void { $this->__d[$o] = null; }
    public function getIterator(): Iterator { return new ArrayIterator($this->__d); }
}

abstract class SplHeap implements Iterator, Countable {
    private $__h = [];
    abstract protected function compare($a, $b);
    public function insert($v) { $this->__h[] = $v; usort($this->__h, fn($a, $b) => $this->compare($b, $a)); return true; }
    public function extract() { if (!$this->__h) throw new RuntimeException("Can't extract from an empty heap"); return array_shift($this->__h); }
    public function top() { if (!$this->__h) throw new RuntimeException("Can't peek at an empty heap"); return $this->__h[0]; }
    public function isEmpty() { return !$this->__h; }
    public function count(): int { return count($this->__h); }
    public function rewind(): void {}
    public function valid(): bool { return (bool)$this->__h; }
    public function current(): mixed { return $this->__h[0] ?? null; }
    public function key(): mixed { return count($this->__h) - 1; }
    public function next(): void { array_shift($this->__h); }
}
class SplMinHeap extends SplHeap { protected function compare($a, $b) { return $b <=> $a; } }
class SplMaxHeap extends SplHeap { protected function compare($a, $b) { return $a <=> $b; } }

class SplPriorityQueue implements Iterator, Countable {
    private $__h = [];
    private $__s = PHP_INT_MAX;
    public function compare($a, $b) { return $a <=> $b; }
    public function insert($v, $p) { $this->__h[] = [$v, $p, $this->__s--]; usort($this->__h, fn($x, $y) => ($r = $this->compare($y[1], $x[1])) !== 0 ? $r : $y[2] <=> $x[2]); return true; }
    public function extract() { if (!$this->__h) throw new RuntimeException("Can't extract from an empty heap"); return array_shift($this->__h)[0]; }
    public function top() { if (!$this->__h) throw new RuntimeException("Can't peek at an empty heap"); return $this->__h[0][0]; }
    public function isEmpty() { return !$this->__h; }
    public function count(): int { return count($this->__h); }
    public function rewind(): void {}
    public function valid(): bool { return (bool)$this->__h; }
    public function current(): mixed { return $this->__h[0][0] ?? null; }
    public function key(): mixed { return count($this->__h) - 1; }
    public function next(): void { array_shift($this->__h); }
}
