#!/bin/sh
# 构建 ia-console deb 包（无 dpkg-deb 环境用 ar+tar 手工组装，结构符合 deb(5)）
# 用法：scripts/build-deb.sh  （在仓库根执行；需先 cargo build --release）
set -e
cd "$(dirname "$0")/.."

VERSION=0.2.1
ARCH=amd64
STAGE=dist/deb-stage
OUT=dist/ia-console_${VERSION}_${ARCH}.deb

rm -rf "$STAGE" "$OUT"
mkdir -p "$STAGE/DEBIAN" \
         "$STAGE/usr/bin" \
         "$STAGE/usr/share/applications" \
         "$STAGE/usr/share/icons/hicolor/256x256/apps" \
         "$STAGE/usr/share/doc/ia-console"

# ---- data ----
install -m 755 target/release/ia-console "$STAGE/usr/bin/ia-console"
install -m 755 target/release/iac        "$STAGE/usr/bin/iac"
install -m 755 target/release/iacd       "$STAGE/usr/bin/iacd"
install -m 644 packaging/ia-console.desktop "$STAGE/usr/share/applications/ia-console.desktop"
python3 scripts/gen-icon.py "$STAGE/usr/share/icons/hicolor/256x256/apps/ia-console.png"
install -m 644 README.md "$STAGE/usr/share/doc/ia-console/README.md"
install -m 644 LICENSE "$STAGE/usr/share/doc/ia-console/copyright"

# ---- control ----
cat > "$STAGE/DEBIAN/control" <<CTRL
Package: ia-console
Version: $VERSION
Architecture: $ARCH
Maintainer: zhaoxuan <zhaoxuan@users.noreply.github.com>
Depends: libc6 (>= 2.31) | libc6
Section: devel
Priority: optional
Homepage: https://github.com/zhaoxuan/iA-console
Description: 天下没有难用的 Console（iA Console 智能控制台）
 独立 GUI 客户端：内置真终端（多标签、交互程序可用）与 AI 侧栏
 （意图提案、错误诊断、方言翻译、方案库、会话持久化、风险闸门）。
 附带 iac（CLI/TUI 前端）与 iacd（守护进程，MCP/IPC 服务）。
CTRL

cat > "$STAGE/DEBIAN/postinst" <<'POST'
#!/bin/sh
set -e
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database /usr/share/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q /usr/share/icons/hicolor || true
fi
if command -v kbuildsycoca6 >/dev/null 2>&1; then
  kbuildsycoca6 --noincremental 2>/dev/null || true
fi
POST
chmod 755 "$STAGE/DEBIAN/postinst"

cat > "$STAGE/DEBIAN/postrm" <<'POST'
#!/bin/sh
set -e
if [ "$1" = purge ] && command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database /usr/share/applications || true
fi
POST
chmod 755 "$STAGE/DEBIAN/postrm"

# ---- 打包（data: xz；control: gz）----
mkdir -p dist
tar -C "$STAGE" -czf dist/control.tar.gz ./DEBIAN
tar -C "$STAGE" -cJf dist/data.tar.xz ./usr
printf '2.0\n' > dist/debian-binary

# ar 成员顺序：debian-binary → control.tar.gz → data.tar.xz
rm -f "$OUT"
ar -rD "$OUT" dist/debian-binary dist/control.tar.gz dist/data.tar.xz
rm -f dist/control.tar.gz dist/data.tar.xz dist/debian-binary

echo "构建完成：$OUT"
ar -t "$OUT"
