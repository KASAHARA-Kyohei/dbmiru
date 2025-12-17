# 現在のマイルストーン

M1

## 次にやること

- 本物の PostgreSQL (ローカル) で `SELECT 1` スモークを行い UI 全体を触ってみる
- M1 実装をウォークスルーして改善点/バックログを整理し、M2 のタスク分解を始める

## メモ

- Metal Toolchain の導入で `cargo clippy --no-deps` まで通るようになった
- gpui + tokio-postgres 連携のランタイム設計は `DbMiruApp` に集約済み
