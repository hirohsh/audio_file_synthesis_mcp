# audio_file_synthesis_mcp 設計書（初版）

## 1. 目的
- 複数話者の独立した音声ファイルを入力として受け取り、1つのモノラル音声ファイルを出力する Rust 製 MCP サーバーを提供する。
- 初版では **WAV / MP3 / FLAC / M4A** を入力対応フォーマットとする。
- WAV は PCM / IEEE float に加えて、`WAVE_FORMAT_EXTENSIBLE`（SubFormat: PCM / IEEE_FLOAT）を受け付ける。

## 2. スコープ
### 2.1 対象
- MCP ツール経由での音声合成実行
- 入力音声のデコード、モノラル化、サンプルレート統一、ミキシング、正規化、WAV 出力
- 合成結果メタデータ（長さ、ピーク値など）の返却

### 2.2 非対象（初版）
- ノイズ除去、話者分離、VAD
- 高度なダイナミクス処理（マルチバンドコンプ等）
- ストリーミング出力

## 3. MCP インターフェース
## 3.1 ツール名
- `synthesize_mono_audio`
- トランスポート: stdio / JSON-RPC 2.0（`Content-Length` フレーミング）

### 3.2 入力スキーマ（案）
```json
{
  "inputs": [
    {
      "speaker_id": "spk1",
      "path": "/abs/path/to/speaker1.wav",
      "gain_db": 0.0,
      "start_ms": 0
    }
  ],
  "output_path": "/abs/path/to/out/mix.wav",
  "target_sample_rate": 48000,
  "normalization": {
    "enabled": true,
    "peak_dbfs": -1.0
  }
}
```

- `inputs`: 話者ごとの音声入力。`path` は絶対パス想定。
- `gain_db`: 話者ごとのゲイン補正（省略時 0.0）。
- `start_ms`: 合成開始オフセット（省略時 0）。
- `target_sample_rate`: 出力サンプルレート（省略時 48000）。
- `normalization.peak_dbfs`: ピーク正規化目標（省略時 -1.0 dBFS）。

### 3.3 出力スキーマ（案）
```json
{
  "output_path": "/abs/path/to/out/mix.wav",
  "sample_rate": 48000,
  "channels": 1,
  "duration_ms": 12345,
  "peak_dbfs": -1.02
}
```

## 4. 音声処理パイプライン
1. **入力検証**
   - `inputs` が空でないことを確認
   - パス存在確認、読み取り可否確認、拡張子と実デコード可否を確認
2. **デコード**
   - WAV（PCM / IEEE float / WAV extensible）/ MP3 / FLAC / M4A を `f32` PCM に変換
3. **チャンネル統合（モノラル化）**
   - 複数チャンネル入力は平均化して 1ch 化
4. **リサンプル**
   - `target_sample_rate` に統一
5. **タイムライン配置**
   - `start_ms` を反映して合成バッファへ配置
6. **ミキシング**
   - 各話者のサンプルに `gain_db` を適用して加算
7. **正規化**
   - 有効時はピーク基準で一括スケーリング
8. **出力エンコード**
   - モノラル 16-bit PCM WAV として保存

## 5. エラーハンドリング方針
- 入力不正（空入力、ファイル未存在、`target_sample_rate` 異常）は `invalid_params` 系で返却
- デコード失敗は入力ファイル単位で原因を含める
- 出力失敗（権限・ディスク不足）は `io_error` として返却
- 正規化不能（全無音など）は成功扱いで `peak_dbfs` に `-inf` 相当の値を返す

## 6. Rust 構成（初版案）
```text
src/
  main.rs                # MCP サーバー起動
  mcp/
    mod.rs
    tools.rs             # synthesize_mono_audio の入出力定義
  audio/
    mod.rs
    decode.rs            # WAV/MP3/FLAC デコード
    downmix.rs           # モノラル化
    resample.rs          # サンプルレート変換
    mix.rs               # タイムライン合成
    normalize.rs         # ピーク正規化
    encode.rs            # WAV 出力
  error.rs               # アプリケーションエラー定義
```

## 7. 依存クレート（実装）
- MCP 通信: `serde`, `serde_json`（stdio JSON-RPC を自前実装）
- MP3 デコード: `minimp3`
- FLAC デコード: `claxon`
- M4A デコード: `symphonia`（`isomp4` + `aac` + `alac`）
- WAV デコード/エンコード: プロジェクト内実装（WAV extensible を含む）
- エラー: プロジェクト内 `AppError`

## 8. テスト方針
### 8.1 ユニットテスト
- チャンネル統合: 2ch -> 1ch の振幅が期待値になること
- リサンプル: 長さとサンプルレートが期待通りであること
- ミキシング: `gain_db` と `start_ms` の反映確認
- 正規化: 目標ピークに収束しクリップしないこと

### 8.2 統合テスト
- WAV（extensible 含む）/ MP3 / FLAC / M4A 混在入力で単一モノラル WAV が生成されること
- 空入力、壊れたファイル、書き込み不可パスで適切なエラーを返すこと

## 9. 今後の実装ステップ
1. MCP サーバースケルトン実装
2. 音声処理モジュール実装（decode -> downmix -> resample -> mix -> normalize -> encode）
3. テストデータ追加と自動テスト整備
