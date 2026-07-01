# スクリーンショット撮影ガイド(portfolio README 用)

README の hero / Demo セクションは、このフォルダ(`docs/assets/`)に置かれた次の
4枚を参照している。**ファイル名は README が固定参照しているので、この名前で保存
すること**(名前が違うと画像が壊れる)。

| ファイル名 | 何を撮るか | 出し方 |
|---|---|---|
| `landing.png` | 起動直後の **startup landing**(block-art mascot + `origin-agent — local-first AI workbench` wordmark + `Type a message to start.`) | `origin` を起動した直後の画面 |
| `ask.png` | **Ask mode** が read-only で答えている様子。左 rail と会話行が見える状態 | `/mode ask` → 例:`README.md を3行で要約して` を送信し、回答が出た画面 |
| `agent-review.png` | **Agent review panel**(`1.` generate preview / `2.` adjust / `3.` cancel の番号付きアクション) | `/mode agent` → 例:`README.md の誤字を直して` を送信し、review panel が出た画面(まだ `1` を押す前) |
| `proposal-review.png` | **Proposal review = save permission gate**(`y / n` と target、できれば diff が見える) | 上の続きで `1` を押して preview を生成し、`y / n` の save 承認 panel が出た画面 |

最低限なら `landing.png` と `proposal-review.png` の2枚でも「綺麗な入口」と「人間承認
の安全モデル」という核は伝わる。4枚あると Ask→Agent→承認の流れが完結する。

## 撮り方のコツ

- ターミナルは **80〜100 桁・暗背景** 程度が読みやすい。フォントは少し大きめに。
- 端末文字が潰れないよう、撮影前にウィンドウを十分広げる(landing は背の低い端末
  だと text fallback になるので、ある程度の高さを確保する)。
- **API key / 実際のホスト名 / 個人パスが写り込まないこと**を確認してから保存する
  (config やプロンプトに秘密が出ていないか目視)。
- 実際の応答を見せたい場合は LM Studio をローカルで起動しておくと `ask.png` が
  本物の返答になる。動かせない場合は Ask の入力直後でも可。
- macOS: `Cmd+Shift+4` → スペースでウィンドウ単位の撮影が手早い。保存後に
  `docs/assets/<上記の名前>.png` へリネーム/移動する。

撮り終えたら `git add docs/assets/*.png` で stage(`git add .` は使わない)。
