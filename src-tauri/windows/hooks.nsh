; NSIS hooks (bundle.windows.nsis.installerHooks).

!macro NSIS_HOOK_POSTUNINSTALL
  ; The app registers its toast identity at runtime (notify::register_aumid);
  ; the stock uninstaller doesn't know about that key or the icon it drops.
  DeleteRegKey HKCU "Software\Classes\AppUserModelId\com.skim.app"
  Delete "$PROFILE\.skim\notify-icon.png"
  RMDir "$PROFILE\.skim"
!macroend
