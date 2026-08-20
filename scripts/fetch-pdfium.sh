#!/usr/bin/env bash
# Downloads a prebuilt pdfium into vendor/pdfium/, where pedro-pdf looks for it.
#
# pdfium is a shared library rather than a crate, so it has to be fetched
# separately. The binaries come from bblanchon/pdfium-binaries, which is what
# the pdfium-render documentation points at.
set -euo pipefail

VERSION="${PDFIUM_VERSION:-latest}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DESTINATION="$ROOT/vendor/pdfium"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)  ARCHIVE="pdfium-mac-arm64.tgz" ;;
  Darwin-x86_64) ARCHIVE="pdfium-mac-x64.tgz" ;;
  Linux-aarch64) ARCHIVE="pdfium-linux-arm64.tgz" ;;
  Linux-x86_64)  ARCHIVE="pdfium-linux-x64.tgz" ;;
  *)
    echo "no prebuilt pdfium for $(uname -s)-$(uname -m)" >&2
    exit 1
    ;;
esac

if [ "$VERSION" = "latest" ]; then
  URL="https://github.com/bblanchon/pdfium-binaries/releases/latest/download/$ARCHIVE"
else
  URL="https://github.com/bblanchon/pdfium-binaries/releases/download/$VERSION/$ARCHIVE"
fi

echo "fetching $URL"
mkdir -p "$DESTINATION"
curl --fail --location --silent --show-error "$URL" | tar -xz -C "$DESTINATION"

echo "pdfium is in $DESTINATION/lib"
