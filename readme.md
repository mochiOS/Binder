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

StatusArea (the logo and right-hand status group) appears above windows. By
default it slides away when a visible window comes within 8px below its 54px
strip. Moving the pointer into the top 9px reveals it; moving into the revealed
strip keeps it open. Open shell menus retain it, while an active window
drag/resize retains pointer ownership. Right-click empty space in StatusArea
to choose **Always show** or **Hide when windows overlap**. This is saved as
`status-area=always` / `status-area=automatic` in the same preferences file.

Maximized window outlines, clips and corner hit tests use radius zero, and
restore the usual continuous corners when returned to their previous size.
