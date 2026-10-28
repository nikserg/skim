---
title: Windows 向けオープンソースのメールクライアント — 2026 年の現実的な短いリスト
description: 2026 年に実際にメンテされている Windows 向けオープンソースのメールクライアント 5 つ、ライセンス・認証情報の保管場所、そして「オープンソース」にアスタリスクが付く時。
date: 2026-10-24
updated: 2026-10-28
image: https://skim-tech.com/social-preview.jpg
related: [best-free-email-client-for-windows-11, thunderbird-alternatives-in-2026, best-email-client-for-windows-2026]
---

メールクライアントのランディングページの「オープンソース」は意味が様々です。時には「クライアントは MIT、サーバーなし、コードを読んでください」。時には「クライアントは開かれているが、便利な機能（検索、送信遅延、既読通知）は我々のサーバーで動き、認証情報もそこにある」。どちらも正当ですが同じではありません。

## 確認すべきこと

三つの質問：認証情報はどこに？サーバーコンポーネントもオープンソース？今もメンテされている（最終コミット日）？

## 短いリスト

### Thunderbird（MPL 2.0）

事実上の標準。すべてローカル、ベンダーのサーバーはメール経路になし。Supernova と 2026 の統合受信箱。アイドル 300–500 MB。

### Betterbird（MPL 2.0）

コミュニティフォーク、修正が速い。完全なオープンソース、サーバーなし。

### Skim（MIT）

[Skim](/) は Windows 向けネイティブ Rust のメールクライアント、UI は Svelte 5 と WebView2。~5 MB のインストーラ、管理者権限不要。IMAP で FTS5 検索付きのローカル SQLite へ；すべての変更はローカルの offline-first キュー経由, アーカイブ/削除/送信は UI 上で即時完了し、後でサーバーと整合。

[AI 機能](/#ai) は BYOK かつローカル対応：Anthropic / OpenRouter / OpenAI 互換エンドポイント、または `localhost:11434` の Ollama/LM Studio。認証情報と API キーは Windows 資格情報マネージャーへ。

範囲：メールのみ。カレンダー、連絡先、フィルタ、PGP なし。

リポジトリ：github.com/nikserg/skim。ライセンス：MIT。

### Mailspring（クライアント GPL-3.0）

クライアントはオープン、Pro バックエンドはクローズでセルフホスト不可。

### Claws Mail（GPL-3.0）

Linux の古典、Windows でも動作。GTK、2008 風の美学。

## 比較

| クライアント | ライセンス | ネイティブ UI | クラウド | セルフホスト可 | メールのみ |
|---|---|---|---|---|---|
| Thunderbird | MPL 2.0 | はい（Gecko） | いいえ | N/A | いいえ |
| Betterbird | MPL 2.0 | はい（Gecko） | いいえ | N/A | いいえ |
| Skim | MIT | はい（Rust+WebView2） | いいえ | N/A | はい |
| Mailspring | GPL-3.0（クライアント） | Electron | Pro に必要 | いいえ | はい |
| Claws Mail | GPL-3.0 | はい（GTK） | いいえ | N/A | ほぼ |

## オープンソースがここで重要な理由

メールはあなたのデジタル全体のパスワードリセットの経路です。クローズドなクライアントは, 意図的かどうかに関わらず, 認証情報を漏らしたりサーバー側で索引化したりし得ます。ネイティブで開かれ、ローカルファーストのクライアント（Thunderbird、Betterbird、Skim）は信頼面を、読めるアプリと呼べる API に絞ります。

## FAQ

**2026 年 Windows 向け最良のオープンソースメールクライアント？** 何でもやるなら Thunderbird、速さなら Skim、Thunderbird-ゴツゴツ無しなら Betterbird。

**Windows の Thunderbird ポータブル版？** Portableapps.com。

**新しい Outlook はオープンソース？** いいえ。

**Skim は Thunderbird のような拡張エコシステムを持つ？** いいえ、意図的に。

**Mailspring のバックエンドはセルフホスト可？** いいえ、クライアントのみオープン。

**Thunderbird に AI は？** 組み込みなし。アドオンあり。Skim は自分のキーで製品内に AI を持つ, [AI](/#ai)。
