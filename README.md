# yomitori

画面の英語を OCR で読み取り、日本語に訳して表示する Windows の常駐ツール。翻訳には Google 翻訳を使っている

## 特徴

- Win+Q を押してマウスを動かし、クリックするだけで範囲を読んで訳す
  - 右ボタンを押しながら動かすと、枠の大きさを保ったまま移動できる
- Win+W でカーソルのある 1 行だけを読む
- OCR は Windows 標準のものを使う
- 範囲指定中も結果ウィンドウを出す時も、作業中のウィンドウのフォーカスを奪わない

## 使い方

起動するとタスクトレイに常駐する

### Win+Q: 範囲指定

1. Win+Q を押すと、カーソル位置を起点に枠が出る。マウスを動かすと枠が広がる
2. 右ボタンを押したまま動かすと、枠が移動する
3. 左クリックで確定。ほぼ動かさずに左クリックするか、Esc で中止

- 範囲指定中に Win+Q を押すとカーソル位置からやり直し、Win+W を押すとカーソル行を読む
- 範囲指定中に他のキーを押すか、他のウィンドウが前に出ると中止する

### Win+W: カーソル行

カーソルのある行を読み、訳が届いたら結果ウィンドウを出す

カーソルが行のすぐ近くにある時だけ読む

### 結果ウィンドウ

上段の文字列を書き換えて「Re-translate」で訳し直せる。「Topmost」で常に手前に出す

文字が読めなかった時は結果ウィンドウを出さず、カーソルの近くに `(no text)` を出す

### トレイのメニュー

右クリックで「設定ファイルを開く」「設定を再読み込み」「終了」

## 動作環境

- Windows 10 / 11
- Windows に英語の言語（OCR）が入っている必要がある

## 入れ方

- Release の zip を展開して `yomitori.exe` を起動する
- 自動で起動したい時は、`shell:startup` のフォルダにショートカットを置く
- アンインストールはフォルダを消すだけ（設定ファイルも exe の隣にある）

## 設定ファイル

exe と同じフォルダの `settings.json`。書き換えたらトレイの「設定を再読み込み」で反映する

```json
{
  "hotkeys": { "region": "Win+Q", "line": "Win+W" },
  "showStartupNotification": true,
  "keepLineBreaks": true,
  "resultWindow": { "left": null, "top": null, "width": 600, "height": 200 }
}
```

| 項目 | 意味 |
|---|---|
| `hotkeys.region` | 範囲指定のホットキー |
| `hotkeys.line` | カーソル行のホットキー |
| `showStartupNotification` | 起動時にホットキーの案内を出すか |
| `keepLineBreaks` | 範囲指定で読んだ複数行を改行のまま残すか。`false` なら空白でつないで 1 段落にする |
| `resultWindow.left` / `top` | 結果ウィンドウの位置。`null` なら画面中央 |
| `resultWindow.width` / `height` | 結果ウィンドウの大きさ。幅 300 以上・高さ 200 以上 |

- 結果ウィンドウの位置と大きさは、閉じた時と終了時に保存される。単位は物理ピクセル
- コメントと末尾のカンマは書けない（書くと壊れている扱いで退避される）

### ホットキーの書式

`修飾+修飾+キー`。修飾は `Win` / `Ctrl` / `Alt` / `Shift`。修飾なしの単独キー（例: `F8`）も書ける。大文字小文字は問わない

- 書いた修飾がちょうど押されている時だけ反応する（`Win+Q` は Ctrl+Win+Q では反応しない）
- `region` と `line` を同じにはできない
- 修飾なしの単独キーにすると、そのキーは他のアプリで使えなくなる

使えるキー名:

- `A` 〜 `Z`、`0` 〜 `9`、`F1` 〜 `F24`
- `Space` `Insert` `Delete` `Home` `End` `PageUp` `PageDown` `Left` `Up` `Right` `Down` `PrintScreen` `Pause`

`Esc` は範囲指定の中止に使うので割り当てられない

### 壊れている時

JSON として読めない・値が不正な時は、`settings.json.bak_yyyyMMdd_HHmmss` に退避して既定値に戻し、トレイで知らせる。欠けた項目は既定値で補う

## 注意

- 翻訳には Google 翻訳の非公式のエンドポイント（`translate.googleapis.com` の `client=gtx`）を使っている。予告なく使えなくなることがあり、読み取った文字列は Google に送られる
- 管理者権限のウィンドウが前面にある間は、ホットキーが効かない
- yomitori の動作中は、Win+Q・Win+W（Windows の検索・ウィジェット）を yomitori が受ける
- OCR は英語だけ、訳は英語 → 日本語だけ

## ビルド

Rust のツールチェーン（MSVC）が要る

```
cargo build --release
cargo test
```

できる物は `target/release/yomitori.exe`

アイコンを直す時は `assets/icon/` のドット絵の txt を直し、`pwsh -File assets/icon/make-ico.ps1` で `yomitori.ico` を作り直す

## ライセンス

MIT（[LICENSE](LICENSE)）
