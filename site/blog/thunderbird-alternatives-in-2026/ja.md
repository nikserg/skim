---
title: 2026 年の Thunderbird 代替 — Windows 向け 6 つの正直な選択肢
description: Thunderbird を試して合わなかった？Windows 向け 6 つの代替、各々が Thunderbird のワークフローから何を残し何を落とすか、どの不満を実際に解決するか。
date: 2026-10-23
updated: 2026-10-28
image: https://skim-tech.com/social-preview.jpg
related: [best-free-email-client-for-windows-11, open-source-email-client-for-windows, outlook-alternatives-for-windows]
---

Thunderbird は「Windows 上の無料・オープンソースのメールクライアント」の標準回答で、たいてい正解です。しかし「Thunderbird の代替」を探す理由はたいてい四つ：重い；Supernova 後も UI が密；特定のバグが人を追い出す；アドオンではなく組み込みの AI がほしい。

## 六つの代替

### 1. Betterbird, バグなしの Thunderbird

コミュニティフォーク、修正が速い。同じ機能、少ない摩擦。

### 2. Skim, 速く軽く AI ネイティブ

[Skim](/) は Windows 向けの無料・オープンソースのメールクライアントで、Gecko ではなくネイティブ Rust コアを採用。~5 MB のインストーラ、管理者権限不要、コールドスタートは一瞬。AI コパイロットはあなたの API キーで. Anthropic、OpenRouter、OpenAI 互換エンドポイント、[Ollama のローカルモデル](/#ai), サブスクなし。

解決：重さ、起動、サブスク無しの組み込み AI。落とす：カレンダー、連絡先、フィルタ、PGP。

### 3. eM Client, Outlook 形、2 アカウントまで無料

クローズドだが個人の無料層あり。カレンダー、連絡先、チャット、タスクが一窓に。

### 4. Mailspring, 綺麗、クラウド

オープンソースのフロントエンド、バックエンドは Mailspring のサーバーで IMAP 認証情報を保持。

### 5. Spark Desktop, 「スマート」受信箱

グループ化と優先度付け。有料で AI。メールは Readdle 経由。

### 6. Claws Mail, Linux 古典、Windows でも

GTK、高度にカスタム可能、2008 風美学。

## 比較

| 代替 | どの不満を解決 | オープンソース | ネイティブ | クラウド | カレンダー | AI |
|---|---|---|---|---|---|---|
| Betterbird | 特定バグ | はい | はい（Gecko） | いいえ | はい | いいえ |
| Skim | 重さ、起動、組み込み AI なし | はい | はい（Rust+WebView2） | いいえ | いいえ | BYOK |
| eM Client | UI の密 | いいえ | はい | いいえ（コア） | はい | アドオン |
| Mailspring | UI の密 | コア | いいえ（Electron） | Pro で必要 | いいえ | いいえ |
| Spark | レイアウト | いいえ | いいえ（Electron） | 必須 | はい | 有料 |
| Claws Mail | UI の重さ | はい | はい（GTK） | いいえ | いいえ | いいえ |

## 不満別の選択

- 「Thunderbird があるとこで落ちる」→ Betterbird。
- 「遅い、カレンダー/連絡先は不要」→ Skim。
- 「UI が密すぎ」→ eM Client。
- 「アドオンではなく、メールに AI」→ Skim（BYOK）か Spark（同梱）。
- 「見た目を現代的に」→ eM Client か Mailspring。
- 「Thunderbird はいいが軽い版が欲しい」→ Betterbird。

## FAQ

**組み込み AI 付きの Thunderbird フォークは？** 公式にはなし。BYOK 組み込み AI：[Skim](/)。

**Skim は Thunderbird のメールを取り込む？** メールはサーバーから同期, プロファイルの取り込みは不要。

**Mailspring と Spark は本当にオープンソース？** Mailspring クライアントははい、バックエンドはいいえ。Spark は完全クローズド。

**Betterbird は合法？** はい、MPL 2.0 のフォーク。

**Windows の Thunderbird ポータブル版？** あり、portableapps.com。Skim は管理者権限なしで入る, 同じ用途。
