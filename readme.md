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
