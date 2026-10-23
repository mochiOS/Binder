# Binder

Desktop environment / file manager for mochiOS.

## Status items

The gray status pill opens Control Center. Date/time remains a separate calendar
target; input method and notification icons retain their direct actions.

System providers and application declarations use AppCore's `StatusItemRegistry`.
Applications keep declaring `[[application.control_center_items]]` in their
format-1 manifest (`id`, `title`, VK Symbols asset `symbol`, and `action` equal
to `open-application` or `show-card`). A live card is published through the
existing AppCore `ControlCenterItem`; registration does not grant capabilities.
Application IDs are `app.<bundle-id>:<item-id>` and system IDs are `builtin.*`.

Control Center's **Edit status items** opens the shared placement/order editor:

- **Hidden**: absent from both menu and status bar.
- **Menu**: available in Control Center.
- **Pinned**: available in both places, subject to available status-bar space.

User preferences override provider defaults. System indicators default to Pinned;
application items default to Menu. Overflow remains accessible in Control Center.
Preferences retain the existing `control-center.conf` path and `item=` ordering,
and accept `hidden=`, `menu=` and `pinned=` placement records. Existing hidden
preferences are preserved.

OSMenu's logo and the right-hand StatusArea appear above windows, with
independent overlap tests and reveal animations. Moving into the top 9px above
either region reveals only that region; moving into its revealed strip keeps it
open. The open OS menu retains the left logo, while Control Center and
notifications retain only the right group. Window drag/resize retains pointer
ownership. Right-click empty space in either revealed region
to choose **Always show** or **Hide when windows overlap**. This is saved as
`status-area=always` / `status-area=automatic` in the same preferences file.

The right group samples its underlying background at five points, at most ten
times per second. Light backgrounds use black text/symbols on a translucent light
surface; dark backgrounds use white text/symbols on a translucent dark surface.
The surface appears only while the pointer is over the group. At rest, only
text and symbols remain; opening a menu does not keep the surface highlighted.
Luminance hysteresis avoids rapid switching near the threshold. The shell's own
surfaces are excluded: Linux preview samples the underlying display list and
mochiOS queries the compositor's underlying surfaces through the privileged
decoration connection, without a GPU framebuffer readback.

Maximized window outlines, clips and corner hit tests use radius zero, and
restore the usual continuous corners when returned to their previous size.
