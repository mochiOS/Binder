# Kinako desktop: static appearance refinement

## Scope and decisions

The October 31 native desktop captures and the desktop/ViewKit code were audited
before changing code. The problem was fragmented rendering and surface rules,
not a reason to replace the continuous-corner model or the existing assets.

| Area | Correction |
| --- | --- |
| Omochi at rest | Use exactly the same continuous fill, inside border and shadow commands as Rectangle. |
| Omochi during deformation | Offset along contour normals; use a proper contour stroke and the same bounded shadow profile as Rectangle. No nonuniform shape scaling or fixed five-layer shadow recipe. |
| Floating UI | OS menus, Applications, search, task switching, notification surfaces and control centre use Surface::floating material. Preserve radii by role, including 22px launcher/control panels. |
| Control centre | One material across grid/detail pages, inset the grid within a padded floating frame, match visible boundary and input, and use theme colours for secondary text. |
| Window chrome | Keep 40px title bar and existing controls; use native label weight/ink centring and retain title space in narrow utility windows. Make shell backing opaque and soften the separator. |
| Native window elevation | ViewKit and compositor read the same dependency-free window_elevation.rs data. Preserve the 20px damage budget and window corner radius. |
| Dock | Preserve the mock geometry, icon assets, item layout and 10px bottom margin. Raise material opacity from 128 to 208, add the existing restrained dock shadow and inside outline. |
| Status area | Preserve logo, assets, position, icon dimensions and background-dependent black/white foreground. Share text/row metrics and native label weight; reject clicks outside the visible rounded background. No permanent bar. |
| OS-menu density | Use existing 13px control text and 24px small-control rows. Derive popup height from actual items/groups/padding instead of a fixed 144px frame. |

Neutral surface colours remain sourced from the existing theme. Accent, font
families, VK Symbols, wallpaper files, logo and application icons are unchanged.
The opaque working-window backing and shared elevation distinguish the working
plane from the three-dimensional wallpaper; no wallpaper dimming layer, backdrop
blur, gradient or replacement image is added.

These values are this refinement's implementation choices, not retroactively
claimed original OS specifications. The ui-ux-pro-max search was used as an audit
aid; its web layout, palette, font and icon recommendations were not adopted.

## Geometry, redraw and performance

Surface.radius applies the same radius to fill, border, clipping and cutout input
rejection. Compact/normal menus keep their respective corner roles and share the
floating material. SurfaceTokens.shadow_outset derives popup damage from the
actual configured shadow. Control-centre navigation accounts for both page
frames and existing hover labels. Auto-hide, window priority and popup input
transforms remain in place.

Resting controls use batched rounded-rectangle commands rather than triangulated
polygons. Shared shadow coverage uses at most 32 layers and preserves total
alpha. Window elevation stays inside the existing native damage margin; no
per-pixel integration or new shader/atlas image is introduced.

## Verification

Tests cover resting command equivalence in light/dark themes, constant normal
border offsets, bounded shadow alpha, floating fill/border/clip/input agreement,
shell material aliases, narrow title allocation, title typography and theme,
control-centre grid/detail material and hit shape, and native damage extents.
Existing desktop auto-hide, menu animation and coordinate-transform tests remain
part of the validation. Native Binder/compositor builds use workspace-local POSIX
client/protocol overrides to avoid mixing a cached Git client with the current
workspace syscall ABI. UI interaction and subjective appearance review remain
with the user; passing render/geometry tests is not presented as visual approval.
