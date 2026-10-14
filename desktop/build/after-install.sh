#!/bin/bash
# electron-builder 的 deb.afterInstall 会整体替换内置模板，所以这里保留模板原有的
# chrome-sandbox 权限与 desktop 数据库更新逻辑，再追加 AgentPocket 自己的部分。
# 模板来源：app-builder-lib/templates/linux/after-install.tpl

if type update-alternatives >/dev/null 2>&1; then
    # Remove previous link if it doesn't use update-alternatives
    if [ -L '/usr/bin/${executable}' -a -e '/usr/bin/${executable}' -a "`readlink '/usr/bin/${executable}'`" != '/etc/alternatives/${executable}' ]; then
        rm -f '/usr/bin/${executable}'
    fi
    update-alternatives --install '/usr/bin/${executable}' '${executable}' '/opt/${sanitizedProductName}/${executable}' 100 || ln -sf '/opt/${sanitizedProductName}/${executable}' '/usr/bin/${executable}'
else
    ln -sf '/opt/${sanitizedProductName}/${executable}' '/usr/bin/${executable}'
fi

# Check if user namespaces are supported by the kernel and working with a quick test:
if ! { [[ -L /proc/self/ns/user ]] && unshare --user true; }; then
    # Use SUID chrome-sandbox only on systems without user namespaces:
    chmod 4755 '/opt/${sanitizedProductName}/chrome-sandbox' || true
else
    chmod 0755 '/opt/${sanitizedProductName}/chrome-sandbox' || true
fi

if hash update-mime-database 2>/dev/null; then
    update-mime-database /usr/share/mime || true
fi

if hash update-desktop-database 2>/dev/null; then
    update-desktop-database /usr/share/applications || true
fi

# Install apparmor profile. (Ubuntu 24+)
if apparmor_status --enabled > /dev/null 2>&1; then
  APPARMOR_PROFILE_SOURCE='/opt/${sanitizedProductName}/resources/apparmor-profile'
  APPARMOR_PROFILE_TARGET='/etc/apparmor.d/${executable}'
  if apparmor_parser --skip-kernel-load --debug "$APPARMOR_PROFILE_SOURCE" > /dev/null 2>&1; then
    cp -f "$APPARMOR_PROFILE_SOURCE" "$APPARMOR_PROFILE_TARGET"
    if ! { [ -x '/usr/bin/ischroot' ] && /usr/bin/ischroot; } && hash apparmor_parser 2>/dev/null; then
      apparmor_parser --replace --write-cache --skip-read-cache "$APPARMOR_PROFILE_TARGET"
    fi
  else
    echo "Skipping the installation of the AppArmor profile as this version of AppArmor does not seem to support the bundled profile"
  fi
fi

# 随包分发的节点守护进程装到 /usr/bin/agentpocket。
# 旧 Tauri 包用 bundle.linux.deb.files 做同一件事，迁移到 Electron 时丢了，
# 结果 deb 装完 /usr/bin/agentpocket 还是上一版留下的。
BUNDLED_DAEMON="/opt/${sanitizedProductName}/resources/agentpocket"
if [ -f "$BUNDLED_DAEMON" ]; then
    # 该路径由别的包管理时不插手：版本归包管理器决定，本包不越权覆盖。
    if ! dpkg -S /usr/bin/agentpocket >/dev/null 2>&1; then
        install -m 755 "$BUNDLED_DAEMON" /usr/bin/agentpocket
    fi
fi
