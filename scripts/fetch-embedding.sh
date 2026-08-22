#!/usr/bin/env bash
# Downloads the embedding model pedro uses for similarity search into
# vendor/embedding/, where pedro-search looks for it.
#
# hotchpotch/static-embedding-japanese is a *static* embedding: a table of one
# vector per token, so a passage is embedded by looking its tokens up and
# averaging. No transformer, no GPU, no server — which is why it can run inside
# a reader. 134MB, MIT licensed.
#
# Everything works without it; similarity search and the retrieval that feeds
# a question are what it adds.
set -euo pipefail

MODEL="${PEDRO_EMBEDDING_MODEL:-hotchpotch/static-embedding-japanese}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DESTINATION="$ROOT/vendor/embedding"

mkdir -p "$DESTINATION"

for file in 0_StaticEmbedding/model.safetensors 0_StaticEmbedding/tokenizer.json; do
  url="https://huggingface.co/$MODEL/resolve/main/$file"
  echo "fetching $url"
  curl --fail --location --progress-bar "$url" -o "$DESTINATION/$(basename "$file")"
done

echo "the embedding model is in $DESTINATION"
