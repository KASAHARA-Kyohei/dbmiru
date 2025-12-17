# UI — DbMiru

## Layout (M2)

- Left: Connection list (profiles, connection status)
- Center top: Tab bar (`スキーマブラウザ`, `SQLエディタ`)
- Tab `スキーマブラウザ`: schemas → tables → columns → preview を縦に表示
- Tab `SQLエディタ`: editor + 実行ボタン、下部に結果パネル

## Interactions (MVP)

- Select a connection profile → connect
- Write SQL → execute
- Results appear in the SQL tab result panel
- Errors appear inline (connection panel / editor panel / schemaブラウザ)

## Schema browser (M2)

- 接続成功時に自動でスキーマ一覧を読み込み、最初のスキーマ/テーブルを自動選択
- スキーマ/テーブル/カラムのリストは最大 5 件（ウィンドウ高さの約 25%）まで表示し、それ以上はリスト内で縦スクロール
- スキーマ/テーブル名は右クリックでコピー、カラム名は左クリックでコピー
- テーブル選択時にカラム一覧とプレビュー（`SELECT * ... LIMIT 50`）を同タブ内に表示
- メタデータ取得エラーはスキーマブラウザ下部に表示し、UI は落ちない

## SQL editor tab

- SQL 入力・実行ボタン・実行状況を表示
- 実行結果とエラーはタブ内の下部パネルに表示

## Shortcuts

- Cmd/Ctrl + Enter: execute query
- Cmd/Ctrl + W: close tab (when tabs exist)

## UX rules

- Show a running indicator during connect/execute
- Disable execute while a query is running
- Always show feedback (success row count or error message)
