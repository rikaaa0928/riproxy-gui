#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <binary-path> <output-dir> <version>" >&2
  exit 2
fi

binary_path="$1"
output_dir="$2"
version="$3"

app_name="RiProxy"
bundle_id="moe.rikaaa0928.riproxy-gui"
bundle_name="${app_name}.app"
app_dir="${output_dir}/${bundle_name}"
contents_dir="${output_dir}/${bundle_name}/Contents"
macos_dir="${contents_dir}/MacOS"
resources_dir="${contents_dir}/Resources"
iconset_dir="${resources_dir}/AppIcon.iconset"

mkdir -p "${macos_dir}" "${resources_dir}" "${iconset_dir}"
cp "${binary_path}" "${macos_dir}/riproxy-gui"
chmod 755 "${macos_dir}/riproxy-gui"

write_info_plist() {
  local include_icon="$1"
  local icon_plist=""
  if [[ "${include_icon}" == "true" ]]; then
    icon_plist="  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
"
  fi

  cat > "${contents_dir}/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>${app_name}</string>
  <key>CFBundleExecutable</key>
  <string>riproxy-gui</string>
${icon_plist}  <key>CFBundleIdentifier</key>
  <string>${bundle_id}</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>${app_name}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${version#v}</string>
  <key>CFBundleVersion</key>
  <string>${version#v}</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
EOF
}

make_icon() {
  local size="$1"
  local name="$2"
  sips -z "${size}" "${size}" assets/icon.png --out "${iconset_dir}/${name}" >/dev/null
}

if [[ -f assets/AppIcon.icns ]]; then
  cp assets/AppIcon.icns "${resources_dir}/AppIcon.icns"
  write_info_plist true
else
  make_icon 16 icon_16x16.png
  make_icon 32 icon_16x16@2x.png
  make_icon 32 icon_32x32.png
  make_icon 64 icon_32x32@2x.png
  make_icon 128 icon_128x128.png
  make_icon 256 icon_128x128@2x.png
  make_icon 256 icon_256x256.png
  make_icon 512 icon_256x256@2x.png
  make_icon 512 icon_512x512.png
  make_icon 1024 icon_512x512@2x.png

  if iconutil -c icns "${iconset_dir}" -o "${resources_dir}/AppIcon.icns" 2>/dev/null; then
    write_info_plist true
  else
    echo "warning: failed to build AppIcon.icns; packaging app without Finder icon" >&2
    rm -f "${resources_dir}/AppIcon.icns"
    write_info_plist false
  fi
fi
rm -rf "${iconset_dir}"

plutil -lint "${contents_dir}/Info.plist" >/dev/null

if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "${app_dir}" >/dev/null 2>&1 \
    || echo "warning: failed to ad-hoc sign ${bundle_name}" >&2
fi
