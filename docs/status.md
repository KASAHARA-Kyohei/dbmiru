# 現在のマイルストーン

M2

## 次にやること

- pgAdmin 的なメタデータブラウズ体験を洗い出し、schemas/tables/columns/preview の UI 仕様を固める
- `db` モジュールへメタデータ取得 API を追加し、UI から非同期で呼び出せるようにする
- Secure password storage (OS keychain or master password) の方式を調査し、M2 実装方針を決定する

## メモ

- M1 DoD は達成済み（接続/SQL 実行/結果・エラー表示）
- M2 ではメタデータブラウザと安全な資格情報管理を軸にタスク分解を進める
