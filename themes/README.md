# Iceon themes

All shipped application palettes live here as JSON. No Rust knowledge or Rust source edits are needed to modify a palette or add a theme family/variant.

## Create a theme

1. Copy `gitHubDark.json` to a new `.json` file in this directory.
2. Set a unique `id` and `symbol`, then change `family`, `name`, `mode`, `order`, and colors.
3. Keep `"$schema": "./schema.json"` for editor completion and validation.
4. Run `cargo check --workspace --locked` (or ask a maintainer/CI to build your contribution). The build automatically discovers the file, validates it, and generates the theme registry. It appears in both the header dropdown and Preferences previews.

Example metadata: `id: "oceanDark"`, `symbol: "OceanDark"`, `family: "Ocean"`, `name: "Dark"`, `mode: "dark"`, `order: 100`.

`symbol` is a unique PascalCase identifier generated into the app API; it is only metadata, not executable code. Keep existing IDs and symbols stable so saved preferences and existing references continue working. Only the three built-in modes use family `Default`; new palettes should use their own family.

## Palette roles

| Role | Used for |
| --- | --- |
| `background` | Main content/table background |
| `surface` | Sidebars, menus, secondary controls, table headers |
| `elevated` | Hovered/active surfaces and raised controls |
| `foreground` | Main readable text |
| `muted` | Secondary text |
| `border` | Dividers, input borders, outlines |
| `link` | Readable accent text for Name/Namespace and selected navigation labels; use a lighter shade of `primary` on dark themes |
| `primary` | Primary buttons, tinted sidebar highlight, focus rings, selection, progress/slider controls |

Colors must be `#RRGGBB`. Hover/pressed primary shades and high-contrast primary-button text are derived by the renderer. Semantic status colors are shared across all themes in `semantic-tags.json` (danger, success, warning, info; each pair is background then foreground). They are applied after the selected palette, by resolved light/dark mode. Check text/background contrast in both menus and tables; primary-button contrast is covered by UI tests.

`system.json` selects `light.json` or `dark.json` according to OS appearance. Light/Dark retain Iceon's native-material chrome and default component behavior; custom families map the palette roles consistently across component surfaces.

## Build and packaging

Theme JSON is read at **build time** from this directory and embedded in the executable. Changes trigger Cargo rebuilds; restart/rebuild to see edits. This is not runtime loading, hot reload, or arbitrary theme import. Packaged applications work from any directory without shipping loose JSON files. Malformed files, unknown properties, invalid colors, and duplicate identities fail the build. Unknown saved theme IDs are not silently accepted.

Validation: `cargo test -p iceon-core preferences --locked` and `cargo test -p iceon-ui shell_theme --locked`.
