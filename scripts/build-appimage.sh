#!/bin/sh
# 构建 ia-console AppImage（Linux x64）。
# 前置：cargo build --release；官方 type2 runtime 位于 dist/runtime-x86_64
#       （从任一官方 AppImage 前缀抠取，或 GitHub release 下载）；
#       sqfs-helper（backhand 写 gzip squashfs）已构建于 /tmp 或 PATH。
set -e
cd "$(dirname "$0")/.."

APPDIR=dist/AppDir
RUNTIME="${RUNTIME_FILE:-dist/runtime-x86_64}"
HELPER="${SQFS_HELPER:-/tmp/sqfs-helper/target/release/sqfs-helper}"
OUT=dist/ia-console-0.2.3-x86_64.AppImage

rm -rf "$APPDIR" "$OUT"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/256x256/apps"
install -m 755 target/release/ia-console target/release/iac target/release/iacd "$APPDIR/usr/bin/"
cp packaging/ia-console.desktop "$APPDIR/usr/share/applications/ia-console.desktop"
sed -i 's|^Icon=.*|Icon=ia-console|' "$APPDIR/usr/share/applications/ia-console.desktop"
cp scripts/ia-console.png "$APPDIR/usr/share/icons/hicolor/256x256/apps/ia-console.png" 2>/dev/null || \
  python3 scripts/gen-icon.py "$APPDIR/usr/share/icons/hicolor/256x256/apps/ia-console.png"
cat > "$APPDIR/AppRun" <<'APPRUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
case "${1:-}" in
  iac)  exec "$HERE/usr/bin/iac" "${@:2}" ;;
  iacd) exec "$HERE/usr/bin/iacd" "${@:2}" ;;
  *)    exec "$HERE/usr/bin/ia-console" "$@" ;;
esac
APPRUN
chmod +x "$APPDIR/AppRun"
ln -sf ia-console.png "$APPDIR/.DirIcon"
cp "$APPDIR/usr/share/applications/ia-console.desktop" "$APPDIR/ia-console.desktop"
cp "$APPDIR/usr/share/icons/hicolor/256x256/apps/ia-console.png" "$APPDIR/ia-console.png"

"$HELPER" "$APPDIR" "$RUNTIME" "$OUT"
chmod +x "$OUT"
echo "AppImage 构建完成：$OUT"
