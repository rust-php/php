# PHP の Rust 完全リライト計画（rust-php org）

## Context
- php-src の C コードは約 82 MB（約 8,200 万バイト）。`rust-php` org は現在リポジトリ 0 件、`~/dev/rust-php` は空。
- 目的: PHP 本体（Zend エンジン → 拡張 → SAPI）を段階的に Rust へ置換し、メモリ安全性の脆弱性クラスを除く。
- 制約: トークン節約。人手レビュー無しで進められるよう、**php-src の phpt テスト（約 2 万本）を唯一の正解判定**にする。
- 前提確認: `gh` に `read:project` スコープが無い。Projects 作成には `gh auth refresh -s project,read:project` が必要（ユーザー操作）。

## 方針（トークン節約の要）
1. **書き直さず仕様で駆動**: phpt を Rust ハーネスで回し、失敗数を単調に減らす。C を読むのは該当関数のみ（`codemap show`）。
2. **垂直スライスで進める**: lexer → parser → AST → compiler → VM → 最小ランタイム、の順に「`echo 1+1;` が通る」状態を最速で作る。
3. **拡張は後回し・依存順**: standard, pcre, json, mbstring, ctype, date, spl, hash, … を phpt 通過率で優先順位付け。
4. **JIT は最後**: Cranelift 利用で別 Phase。まず `opcache` 無しのインタプリタで互換性 100% を狙う。
5. **差分オラクル**: 本物の `php` バイナリと出力を比較するファズ（cargo-fuzz + 差分テスト）で phpt 以外も補う。
6. サブエージェントはモジュール単位（1 Phase = 1 リポジトリ/クレート）で並列化し、メインの文脈には「通過率の数字」だけ返す。

## リポジトリ構成（rust-php org）
| リポジトリ | 役割 |
| --- | --- |
| `rust-php/php` | Cargo workspace 本体（下記クレート） |
| `rust-php/phpt-runner` | phpt パーサ＋実行ハーネス、通過率レポート |
| `rust-php/.github` | 組織プロフィール、Projects 運用ルール |

クレート: `zend-lex`, `zend-parse`, `zend-ast`, `zend-compile`, `zend-vm`, `zend-mm`（zval/refcount/GC）, `zend-types`（string/array/object）, `php-ext-*`, `php-sapi-cli`, `php-jit`（後期）。

## フェーズ
| Phase | 内容 | 完了条件（測定可能） |
| --- | --- | --- |
| 0 | org/Projects/CI 整備、phpt-runner、php-src を submodule（テスト取得用） | 本物 php で phpt が全件走る |
| 1 | lexer + parser + AST（Zend/zend_language_*.l/y 相当） | Zend/tests の構文系 phpt が parse 成功 |
| 2 | zval/文字列/配列（ordered hash）/refcount/サイクルGC | 型系 unit test + `Zend/tests` の値系 |
| 3 | opcode コンパイラ + VM + 関数/クラス/例外/ジェネレータ/Fiber | `Zend/tests` 通過率 90% |
| 4 | `ext/standard` 優先 + pcre(Rust regex ではなく PCRE2 互換層) + json + ctype + spl | `ext/standard/tests` 85% |
| 5 | date, mbstring, hash, random, filter, tokenizer, reflection | 各拡張 85% |
| 6 | SAPI: cli → cgi/fpm → embed、php.ini、ストリーム/ファイル/ソケット | 実アプリ（Composer, Laravel）起動 |
| 7 | 残り拡張（curl, openssl, intl, pdo 等）は既存 Rust crate をラップ | 優先度別 |
| 8 | JIT（Cranelift）/ opcache 相当 | ベンチで interpreter 比 |

## Projects 作成内容
- 組織 Project「PHP Rust Rewrite」（Board、列: Backlog / In Progress / Blocked / Done）。
- フィールド: Phase, ext 名, phpt 通過率（number）。
- 初期 Issue: 上記 Phase 0〜8 を epic として 9 本、Phase 0〜2 はサブ Issue まで展開。
- 実行は ExitPlanMode 承認後（`gh project create --owner rust-php`、スコープ更新後）。

## 実行順（承認後）
1. ユーザーに `! gh auth refresh -s project,read:project` を依頼。
2. `rust-php/php`, `rust-php/phpt-runner`, `rust-php/.github` を作成（非公開か公開かは下の確認事項）。
3. Project と Phase epic Issue を作成、README に本計画を転記。
4. Phase 0 着手: phpt-runner の骨組みを実装して通過率ベースラインを出す。

## 検証
- `gh project list --owner rust-php` と `gh issue list -R rust-php/php` で作成物を確認。
- Phase 0 完了時: `phpt-runner --php /usr/bin/php php-src/Zend/tests` が全件走り、結果 JSON が出る。

## 確認事項（承認時に決める）
- リポジトリは公開で作ってよいか（脆弱性話題なので公開が自然）。
- ライセンス: PHP License 3.01（本家互換）で進めてよいか。
