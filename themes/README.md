# Iceon themes

Runtime themes use the Zed theme format. Open **Preferences → Themes** to choose
System, Dark, or Light, install Catppuccin/GitHub explicitly, or install another
extension from [Zed's theme marketplace](https://zed.dev/extensions?filter=themes)
by ID or marketplace URL. See [theme management](../docs/preferences.md#themes).

Marketplace theme JSON/JSONC and licenses are stored under Iceon's application
data directory in `themes/zed/<extension-id>/installed.json`. No extension code
is extracted or executed. Install/update is explicit; removal is available on the
Themes page. Unsupported Zed properties keep native GPUI defaults.

The JSON files here define the fallback light/dark palettes and legacy palette
identities used for compatibility tests; only System/Dark/Light appear by default.
Semantic fallback colors are in `semantic-tags.json`. Imported Zed tokens override
the fallbacks. Do not add bundled optional families: use the marketplace instead.
