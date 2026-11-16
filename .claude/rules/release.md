# リリースの手順

新しい版を GitHub の Release に出す時は、上から順に全部通せ

1. `Cargo.toml` の `version` を上げろ。SemVer の 3 つの数字で書け（4 つ目は Cargo が受け付けない）。不具合の修正と小さな変更はパッチを上げる
2. README を実装と突き合わせろ。使う人が知らないと困る変更だけを書き足し、食い違いを直せ。書き方は今の README に合わせる（短く、1 項目 1 文、文末の「。」なし）
3. `cargo clippy --all-targets` の警告 0、`cargo test` 全緑を確かめろ
4. `cargo build --release` で `target/release/yomitori.exe` を作れ
5. 公開する前に、追跡しているファイルと、push するコミットのメッセージを grep して、個人の情報（手元のパス・メールアドレス・所属等）が無いことを確かめろ。テストの架空のパスは除いてよい。公開した物は消しても残るため
6. zip を作れ。中身は `yomitori.exe`・`README.md`・`LICENSE` の 3 つを直下に置く。名前は `yomitori-vX.Y.Z.zip`。リポジトリの外に作る
7. user の合意を取ってから公開しろ
   - `git push origin main`
   - `gh release create vX.Y.Z <zip> --title vX.Y.Z --target main --notes '<変更点>'`。変更点は使う人に見える事だけを短く箇条書き
8. `gh release view vX.Y.Z` で zip が付いていること、`git ls-remote origin refs/heads/main` が手元の main と一致することを確かめろ
