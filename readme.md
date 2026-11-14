# Binder

Desktop environment / file manager for mochiOS.

## Tablet development preview

Run `mmake ui-tablet` from the mochiOS workspace. It uses the same Linux preview
workspace, installed application catalogue, wallpaper, application processes and
AppCore lifecycle as `mmake ui`. It does not build a separate OS image. Desktop
presentation remains the default. Tablet preview fills the host monitor; the
desktop preview retains its existing window behavior. Binder selects its root once at startup with
`--shell-mode=desktop|tablet`, or `BINDER_SHELL_MODE` when no command line mode
is given. The command line takes precedence; automatic device selection belongs
at this same entry point.

Tablet presentation provides a continuous home canvas with half-viewport snaps,
a fixed vertical Dock, searchable application drawer, the existing notification
center and Control Center, and cards for actual running application windows.
Applications fill the logical work area below the status strip. The navigation
bar stays hidden over applications until a bottom-edge slide reveals it briefly;
it overlays content without reserving a permanent bottom margin. Returning home
minimizes their windows, preserving application state.
Closing a switcher card sends the normal application window close requests,
including the application's opportunity to ask about unsaved documents.

- Drag horizontally on home to scroll; release velocity affects the snap target.
- Swipe up on home to open applications; swipe down to open notifications.
- Pull down from the upper right to open Control Center, including while an
  application is in front. The rest of the status strip opens notifications.
- Swipe the drawer down from its top to close it; scroll its grid to browse.
- Swipe up on the notification header, or outside its list, to close it.
- Swipe up from the bottom edge to return home; pause after moving up to open
  the switcher. Swipe along the bottom edge to change running applications.
  Applications extend to the screen bottom; the navigation bar is normally
  hidden and appears temporarily when a bottom-edge swipe starts.
- Hold a home icon to edit, then drag it. Drop in a cell to move/swap, onto an
  icon's center to create/add to a folder, or onto the Dock to pin it. Choose
  Done to leave editing. Tap a folder to open it, or outside it to close it.
- Escape closes the current overlay or editing mode. Mouse dragging exercises
  the same recognizer as touch; tablet preview leaves the host mouse uncaptured.

Icon footprints follow the logical viewport in both orientations. Fullscreen
viewports are negotiated when applications connect and updated immediately on
display size changes. Home navigation carries the finger's position and velocity
into ViewKit's interruptible spring transitions: application size and position
settle independently, and a held swipe moves the actual window into its switcher
card. These interactions follow the iPad presentation model; identical timing
and rendering to iPadOS are not an established compatibility guarantee.

Grid placement stores application identities and integer cell coordinates in
`binder/tablet-home.json`, separate from catalogue metadata. Folder membership
is stored there too. Dock pins retain the existing `dock.conf` format. Scroll,
gesture and overlay state remain transient, with one exclusive overlay scene.
For `ui-tablet`, home placement survives preview sessions at
`out/preview/config/binder/tablet-home.json`; `BINDER_HOME_LAYOUT` overrides this
path for isolated development runs. Preview documents/processes still belong
to the disposable session. On mochiOS the placement path is
`/var/config/applications/org.mochios.binder/tablet-home.json`.

On mochiOS, tablet presentation leases existing Kagami windows through the
capability-checked decoration connection. Logical work areas are converted
using the ViewKit window's actual scale. Native app pixels remain in Kagami;
Binder paints transparent system overlays and registers top/bottom input
sensors. Home hides existing windows, activation restores them, and close
requests go to their original event endpoints. Dead managers release their
leases on the compositor's next event-loop maintenance pass. Protected
presentations retain their geometry until withdrawn.

The native adapter compiles for mochiOS, but native tablet behavior still needs
runtime validation. Native app scaling/screenshot previews and hardware touch
contacts are not implemented by this adapter: native switcher cards show real
app identity without a fabricated screenshot. Interactive app morphing and
per-contact touch input are available in the Linux preview. Unavailable host
hardware controls stay unavailable; no application/notification cards are seeded.

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
