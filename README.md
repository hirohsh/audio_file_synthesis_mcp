# audio_file_synthesis_mcp

複数話者の音声ファイルを1本のモノラル WAV に合成する Rust 製 MCP サーバーです。  
stdio 上で JSON-RPC 2.0 を受け付け、`synthesize_mono_audio` ツールを提供します。

## 対応フォーマット

### 入力
- WAV（PCM / IEEE float）
- WAV (`WAVE_FORMAT_EXTENSIBLE` の SubFormat: PCM / IEEE_FLOAT)
- MP3
- FLAC
- M4A（主に AAC / ALAC）

### 出力
- モノラル 16-bit PCM WAV

## セットアップ

```bash
cargo build --release
```

開発時は以下でも起動できます。

```bash
cargo run
```

## MCP ツール仕様

ツール名: `synthesize_mono_audio`

### 入力パラメータ

```json
{
  "inputs": [
    {
      "speaker_id": "spk1",
      "path": "/abs/path/to/speaker1.m4a",
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

- `inputs` (必須): 入力音声配列（1件以上）
- `inputs[].speaker_id` (必須): 話者ID
- `inputs[].path` (必須): 入力音声の絶対パス
- `inputs[].gain_db` (任意, 既定値 `0.0`): 話者ごとのゲイン
- `inputs[].start_ms` (任意, 既定値 `0`): 合成開始オフセット（ms）
- `output_path` (必須): 出力 WAV パス
- `target_sample_rate` (任意, 既定値 `48000`): 出力サンプルレート
- `normalization.enabled` (任意, 既定値 `true`): 正規化の有効化
- `normalization.peak_dbfs` (任意, 既定値 `-1.0`): 正規化目標ピーク（0以下）

### 出力

```json
{
  "output_path": "/abs/path/to/out/mix.wav",
  "sample_rate": 48000,
  "channels": 1,
  "duration_ms": 12345,
  "peak_dbfs": -1.02
}
```

## 実行例（JSON-RPC / line-delimited）

### 1) ツール一覧

```bash
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
  | cargo run --quiet
```

### 2) 音声合成

```bash
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"synthesize_mono_audio","arguments":{"inputs":[{"speaker_id":"a","path":"/abs/path/a.m4a"},{"speaker_id":"b","path":"/abs/path/b.wav","gain_db":-3.0,"start_ms":250}],"output_path":"/abs/path/out.wav","target_sample_rate":48000,"normalization":{"enabled":true,"peak_dbfs":-1.0}}}}' \
  | cargo run --quiet
```

## MCP クライアント設定例

クライアント側で stdio MCP サーバーとして本バイナリを登録してください。例:

```json
{
  "mcpServers": {
    "audio_file_synthesis_mcp": {
      "command": "/abs/path/to/audio_file_synthesis_mcp/target/release/audio_file_synthesis_mcp",
      "args": []
    }
  }
}
```

## トラブルシュート

- `unsupported extension: m4a`  
  - 古いビルドを参照している可能性があります。`cargo build --release` 後にバイナリパスを再確認してください。

- `WAV format 65534 with 16 bits per sample is not supported`  
  - `WAVE_FORMAT_EXTENSIBLE` の SubFormat が PCM/IEEE_FLOAT 以外だと非対応です。入力WAVのSubFormatを確認してください。

- `invalid params: input path must not contain parent directory components`  
  - セキュリティ上、`..` を含むパスは拒否されます。正規化済み絶対パスを使用してください。

- `decode error: ... contained no decodable frames`  
  - 入力ファイル破損、または対応外コーデックの可能性があります。

## テスト

```bash
cargo test
```
