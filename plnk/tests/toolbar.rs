//! Headless tests for the native toolbar: model, legacy layout, hit testing,
//! icon state rules and compositor animation bookkeeping.
//!
//! The layout numbers come from the legacy `FormCollection` constructor
//! (`src/FormCollection.cs`): the toolbar height is a fraction of the primary
//! screen height, every action button is `0.88 * height` square, pens start at
//! `1.2 * button`, consecutive buttons advance `1.1 * button`, each group gap is
//! `0.8 * button`, and the panel ends half a button after the exit button. The
//! same legacy gaps are inserted even when a whole group is hidden.

use std::time::Duration;

use plnk::assets::{icon_directory_for, tray_icon_filename};
use plnk::gfx::composition::Easing;
use plnk::platform::dpi::{DipPoint, DipRect, DpiMap, PhysicalPoint, PhysicalRect};
use plnk::settings::Settings;
use plnk::ui::animation::{ENTER_DURATION, FAST_EXIT_DURATION, ToolbarAnimator};
use plnk::ui::icons::{Icon, icon_for};
use plnk::ui::toolbar::{ButtonId, ToolbarEvent, ToolbarInteraction, ToolbarLayout, ToolbarModel};

const HEIGHT_DIP: f32 = 64.0;
const EPS: f32 = 0.01;

fn default_model() -> ToolbarModel {
    ToolbarModel::from_settings(&Settings::defaults())
}

fn rect_of(layout: &ToolbarLayout, id: ButtonId) -> DipRect {
    layout
        .button_rect(id)
        .unwrap_or_else(|| panic!("{id:?} must be placed in the layout"))
}

fn close(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() <= EPS
}

#[test]
fn layout_matches_legacy_enabled_button_order() {
    let settings = Settings::defaults();
    let buttons = ToolbarModel::visible_buttons(&settings);
    use ButtonId::*;
    assert_eq!(
        buttons,
        vec![
            Dock,
            Pen(1),
            Pen(2),
            Pen(3),
            Pen(4),
            Pen(5),
            Eraser,
            Pan,
            Pointer,
            InkVisible,
            Snapshot,
            Undo,
            Clear,
            Exit,
        ],
        "visible buttons must follow the legacy FormCollection order"
    );
    assert!(
        !buttons.contains(&Pen(0)),
        "Pen0 is disabled by default (legacy Pen0_Enabled=False)"
    );
    assert!(
        !buttons.contains(&PenWidth),
        "the pen-width button is hidden by default (legacy Pen_Width_Icon=False)"
    );

    let model = default_model();
    let layout = ToolbarLayout::new(&model, HEIGHT_DIP, false);
    let h = HEIGHT_DIP;
    let bs = 0.88 * h; // legacy `gpButtons.Height * 0.88`

    // Dock: left edge of the panel, same size as the action buttons, 0.75 wide.
    let dock = rect_of(&layout, Dock);
    assert!(close(dock.left, 0.0));
    assert!(close(dock.top, 0.07 * h));
    assert!(close(dock.right, 0.75 * bs));
    assert!(close(dock.bottom, 0.07 * h + bs));

    // Pens start 1.2 button widths in (room for the dock button) and sit
    // slightly higher than the action buttons (legacy 0.08 vs 0.07).
    let pen1 = rect_of(&layout, Pen(1));
    assert!(close(pen1.left, 1.2 * bs));
    assert!(close(pen1.top, 0.08 * h));
    for index in 1..5 {
        let left = rect_of(&layout, Pen(index));
        let right = rect_of(&layout, Pen(index + 1));
        assert!(close(right.left, left.left + 1.1 * bs), "pen {index}");
    }

    // Gap after the pens, then eraser/pan/pointer as one group.
    let pen5 = rect_of(&layout, Pen(5));
    let eraser = rect_of(&layout, Eraser);
    assert!(close(eraser.left, pen5.left + 1.1 * bs + 0.8 * bs));
    let pan = rect_of(&layout, Pan);
    let pointer = rect_of(&layout, Pointer);
    assert!(close(pan.left, eraser.left + 1.1 * bs));
    assert!(close(pointer.left, pan.left + 1.1 * bs));

    // Gap after the pointer group; the hidden pen-width button changes nothing.
    let ink_visible = rect_of(&layout, InkVisible);
    assert!(close(ink_visible.left, pointer.left + 1.1 * bs + 0.8 * bs));
    let snapshot = rect_of(&layout, Snapshot);
    let undo = rect_of(&layout, Undo);
    let clear = rect_of(&layout, Clear);
    assert!(close(snapshot.left, ink_visible.left + 1.1 * bs));
    assert!(close(undo.left, snapshot.left + 1.1 * bs));
    assert!(close(clear.left, undo.left + 1.1 * bs));

    // Gap before the exit button; panel width is exit right + half a button.
    let exit = rect_of(&layout, Exit);
    assert!(close(exit.left, clear.left + 1.1 * bs + 0.8 * bs));
    assert!(close(layout.size().0, exit.right + 0.5 * bs));
    assert!(close(layout.size().1, h));

    // Empty gaps and margins are not clickable.
    assert_eq!(layout.hit_test(DipPoint::new(50.0, 30.0)), None);
    assert_eq!(
        layout.hit_test(DipPoint::new(pen5.right + 10.0, 30.0)),
        None
    );
    assert_eq!(layout.hit_test(DipPoint::new(-5.0, 30.0)), None);
    assert_eq!(
        layout.hit_test(DipPoint::new(10.0, h + 5.0)),
        None,
        "below the panel"
    );

    // Visibility rules: enable Pen0/Pen9 and the pen-width button, hide the
    // eraser/pan/pointer/undo buttons. The gaps stay where the legacy code
    // inserts them.
    let mut custom = Settings::defaults();
    custom.pens[0].enabled = true;
    custom.pens[9].enabled = true;
    custom.pen_width_icon = true;
    custom.eraser_icon = false;
    custom.pan_icon = false;
    custom.pointer_icon = false;
    custom.undo_icon = false;
    let buttons = ToolbarModel::visible_buttons(&custom);
    assert_eq!(
        buttons,
        vec![
            ButtonId::Dock,
            ButtonId::Pen(0),
            ButtonId::Pen(1),
            ButtonId::Pen(2),
            ButtonId::Pen(3),
            ButtonId::Pen(4),
            ButtonId::Pen(5),
            ButtonId::Pen(9),
            ButtonId::PenWidth,
            ButtonId::InkVisible,
            ButtonId::Snapshot,
            ButtonId::Clear,
            ButtonId::Exit,
        ]
    );
    let custom_model = ToolbarModel::from_settings(&custom);
    let custom_layout = ToolbarLayout::new(&custom_model, HEIGHT_DIP, false);
    let pen9 = rect_of(&custom_layout, ButtonId::Pen(9));
    let pen_width = rect_of(&custom_layout, ButtonId::PenWidth);
    // Both group gaps survive even when the whole eraser/pan/pointer group is
    // hidden, exactly like the legacy cumulative placement.
    assert!(close(pen_width.left, pen9.left + 1.1 * bs + 1.6 * bs));
    let ink_visible = rect_of(&custom_layout, ButtonId::InkVisible);
    assert!(close(ink_visible.left, pen_width.left + 1.1 * bs));
}

#[test]
fn hit_test_tracks_dpi_and_docking() {
    let model = default_model();
    let layout = ToolbarLayout::new(&model, HEIGHT_DIP, false);

    // Every visible button is hittable at its centre.
    for id in model.buttons() {
        let rect = rect_of(&layout, *id);
        let centre = DipPoint::new(
            (rect.left + rect.right) / 2.0,
            (rect.top + rect.bottom) / 2.0,
        );
        assert_eq!(layout.hit_test(centre), Some(*id), "{id:?}");
    }

    // A physical click at any Windows DPI lands on the same DIP button: the
    // layout is DIP-only and the window geometry is derived per monitor DPI.
    let origin = PhysicalPoint::new(600, 400);
    for dpi in [96u32, 120, 144, 168, 192, 240] {
        let map = DpiMap::from_dpi(dpi);
        let window = layout.window_rect(origin, map);
        for id in model.buttons() {
            let rect = rect_of(&layout, *id);
            let centre = DipPoint::new(
                (rect.left + rect.right) / 2.0,
                (rect.top + rect.bottom) / 2.0,
            );
            let physical = map.dip_to_physical(centre);
            let screen = PhysicalPoint::new(origin.x + physical.x, origin.y + physical.y);
            assert_eq!(
                layout.hit_test_physical(screen, window, map),
                Some(*id),
                "dpi {dpi} {id:?}"
            );
        }
        let expected_width = (layout.size().0 * map.scale()).round() as i32;
        assert_eq!(
            window.width(),
            expected_width,
            "the physical window must scale with DPI ({dpi})"
        );
        assert_eq!(window.left, origin.x);
        assert_eq!(window.top, origin.y);
    }

    // Docked: the window is only the dock strip at the panel's right edge and
    // only the dock button is hittable. The same screen point that hits Exit
    // on the undocked panel hits Dock once docked.
    let docked = ToolbarLayout::new(&model, HEIGHT_DIP, true);
    let map = DpiMap::from_dpi(144);
    let panel = layout.window_rect(origin, map);
    let strip = docked.window_rect(origin, map);
    assert_eq!(
        strip.right, panel.right,
        "the strip is flush with the panel"
    );
    assert_eq!(strip.top, panel.top);
    assert_eq!(strip.height(), panel.height());
    assert_eq!(
        strip.width(),
        (docked.size().0 * map.scale()).round() as i32
    );
    assert!(
        strip.width() < panel.width() / 2,
        "the dock strip is a small part of the panel"
    );

    let dock_rect = docked.button_rect(ButtonId::Dock).expect("dock button");
    let dock_centre = DipPoint::new(
        (dock_rect.left + dock_rect.right) / 2.0,
        (dock_rect.top + dock_rect.bottom) / 2.0,
    );
    assert_eq!(docked.hit_test(dock_centre), Some(ButtonId::Dock));
    assert_eq!(
        docked.hit_test(DipPoint::new(dock_rect.right + 10.0, 30.0)),
        None,
        "nothing besides the dock button exists in the docked strip"
    );

    let probe = DipPoint::new(995.0, 30.0);
    let probe_physical = map.dip_to_physical(probe);
    let probe_screen =
        PhysicalPoint::new(panel.left + probe_physical.x, panel.top + probe_physical.y);
    assert_eq!(
        layout.hit_test(probe),
        Some(ButtonId::Exit),
        "995 DIP is inside the exit button at this panel width"
    );
    assert_eq!(
        docked.hit_test_physical(probe_screen, strip, map),
        Some(ButtonId::Dock),
        "docking moves the same screen point onto the dock strip"
    );
}

#[test]
fn easing_has_exact_endpoints_and_zero_end_velocity() {
    for easing in [
        Easing::SmoothStep,
        Easing::EaseOutCubic,
        Easing::EaseInOutCubic,
    ] {
        assert_eq!(easing.value_at(0.0), 0.0, "{easing:?} start");
        assert_eq!(easing.value_at(1.0), 1.0, "{easing:?} end");
        let near_end = easing.value_at(1.0 - 1e-3);
        assert!(
            (1.0 - near_end).abs() <= 1e-5,
            "{easing:?} must arrive with zero velocity (f(0.999)={near_end})"
        );
        let mut previous = -1.0;
        for step in 0..=1000 {
            let value = easing.value_at(step as f32 / 1000.0);
            assert!(
                value >= previous - 1e-6,
                "{easing:?} must be monotonic at {step}"
            );
            previous = value;
        }
    }
    // The legacy toolbar easing is smoothstep: zero velocity at both ends.
    let smooth = Easing::SmoothStep;
    assert!(smooth.value_at(1e-3) <= 1e-3);
    assert!((smooth.value_at(0.5) - 0.5).abs() <= 1e-6);
    // Linear keeps its velocity, proving the check discriminates.
    assert!((Easing::Linear.value_at(1.0) - Easing::Linear.value_at(0.999) - 1e-3).abs() <= 1e-6);
}

#[test]
fn retargeting_starts_from_current_visual_position() {
    assert_eq!(ENTER_DURATION, Duration::from_millis(300));
    assert_eq!(FAST_EXIT_DURATION, Duration::from_millis(160));

    let start = Duration::from_millis(10_000);
    let hidden = PhysicalPoint::new(1546, 0);
    let shown = PhysicalPoint::new(0, 0);
    let mut animator = ToolbarAnimator::new(hidden);

    let enter = animator.enter(start, shown);
    assert_eq!(enter.from, hidden);
    assert_eq!(enter.to, shown);
    assert_eq!(enter.duration, ENTER_DURATION);
    assert_eq!(enter.easing, Easing::SmoothStep);

    // 150 ms into a 300 ms smoothstep slide the visual is exactly half way.
    let midpoint = start + Duration::from_millis(150);
    let current = animator.position_at(midpoint);
    assert!(current.x > 0 && current.x < hidden.x, "{current:?}");
    assert_eq!(current.x, 773, "smoothstep at t=0.5 is exactly half way");

    // Retargeting mid-animation must start from the current visual position,
    // not from the previously requested target.
    let retarget = animator.fast_exit(midpoint, hidden);
    assert_eq!(retarget.from, current);
    assert_ne!(retarget.from, enter.to);
    assert_eq!(retarget.duration, FAST_EXIT_DURATION);
    assert_eq!(animator.position_at(midpoint), current);

    // The retargeted animation moves towards the new target without a jump.
    let next = animator.position_at(midpoint + Duration::from_millis(1));
    assert!(next.x >= current.x);
    assert!(next.x - current.x <= 8, "{current:?} -> {next:?}");

    // After it finishes, a new animation starts from the settled target.
    let finished = midpoint + FAST_EXIT_DURATION + Duration::from_millis(10);
    assert_eq!(animator.position_at(finished), hidden);
    let dock = animator.dock_to(finished, PhysicalPoint::new(900, 0));
    assert_eq!(dock.from, hidden);
    assert_eq!(dock.duration, ENTER_DURATION);
}

#[test]
fn icons_follow_legacy_active_and_highlighter_rules() {
    let mut model = default_model();
    // Legacy picks the highlighter bitmap when `Transparency >= 100`, i.e.
    // alpha <= 155. Pen 5 (alpha 80) is the default highlighter, and the
    // legacy initial selection is pen 1.
    assert_eq!(model.active(), Some(ButtonId::Pen(1)));
    assert_eq!(
        icon_for(ButtonId::Pen(1), &model, false),
        Icon::PencilActive
    );
    assert_eq!(icon_for(ButtonId::Pen(5), &model, false), Icon::Highlighter);
    model.set_active(None);
    assert_eq!(icon_for(ButtonId::Pen(1), &model, false), Icon::Pencil);
    assert_eq!(icon_for(ButtonId::Pen(5), &model, false), Icon::Highlighter);

    model.set_active(Some(ButtonId::Pen(1)));
    assert_eq!(
        icon_for(ButtonId::Pen(1), &model, false),
        Icon::PencilActive
    );
    assert_eq!(
        icon_for(ButtonId::Pen(5), &model, false),
        Icon::Highlighter,
        "only the selected pen shows the active bitmap"
    );

    model.set_active(Some(ButtonId::Pen(5)));
    assert_eq!(
        icon_for(ButtonId::Pen(5), &model, false),
        Icon::HighlighterActive
    );
    assert_eq!(icon_for(ButtonId::Pen(1), &model, false), Icon::Pencil);

    model.set_active(Some(ButtonId::Eraser));
    assert_eq!(
        icon_for(ButtonId::Eraser, &model, false),
        Icon::EraserActive
    );
    assert_eq!(icon_for(ButtonId::Pointer, &model, false), Icon::Pointer);
    model.set_active(Some(ButtonId::Pointer));
    assert_eq!(
        icon_for(ButtonId::Pointer, &model, false),
        Icon::PointerActive
    );
    assert_eq!(icon_for(ButtonId::Pan, &model, false), Icon::Pan);
    model.set_active(Some(ButtonId::Pan));
    assert_eq!(icon_for(ButtonId::Pan, &model, false), Icon::PanActive);

    assert_eq!(
        icon_for(ButtonId::InkVisible, &model, false),
        Icon::InkVisible
    );
    model.set_ink_visible(false);
    assert_eq!(
        icon_for(ButtonId::InkVisible, &model, false),
        Icon::InkHidden
    );

    assert_eq!(icon_for(ButtonId::Dock, &model, false), Icon::Dock);
    assert_eq!(icon_for(ButtonId::Dock, &model, true), Icon::DockBack);
    assert_eq!(icon_for(ButtonId::Exit, &model, false), Icon::Exit);
    assert_eq!(icon_for(ButtonId::Snapshot, &model, false), Icon::Snapshot);
    assert_eq!(icon_for(ButtonId::Undo, &model, false), Icon::Undo);
    assert_eq!(icon_for(ButtonId::Clear, &model, false), Icon::Clear);
    assert_eq!(icon_for(ButtonId::PenWidth, &model, false), Icon::PenWidth);
}

#[test]
fn drag_threshold_suppresses_click_like_legacy() {
    let bounds = PhysicalRect::from_xywh(0, 0, 2560, 1600);
    let panel = (1000, 96);
    let press = PhysicalPoint::new(1200, 800);
    let origin = PhysicalPoint::new(300, 700);
    let mut interaction = ToolbarInteraction::default();

    // Moving up to 20 px is still a click (legacy checks per axis, > 20).
    interaction.begin(Some(ButtonId::Pen(2)), press, origin, true);
    assert_eq!(
        interaction.drag_to(PhysicalPoint::new(1219, 819), bounds, panel),
        None
    );
    assert_eq!(
        interaction.end(Some(ButtonId::Pen(2))),
        ToolbarEvent::Click(ButtonId::Pen(2))
    );

    // Past the threshold the toolbar follows the mouse and the click is
    // suppressed on release, exactly like the legacy `ToolbarMoved` flag.
    interaction.begin(Some(ButtonId::Pen(2)), press, origin, true);
    assert_eq!(
        interaction.drag_to(PhysicalPoint::new(1221, 800), bounds, panel),
        Some(PhysicalPoint::new(321, 700))
    );
    assert_eq!(
        interaction.drag_to(PhysicalPoint::new(1500, 900), bounds, panel),
        Some(PhysicalPoint::new(600, 800))
    );
    assert_eq!(
        interaction.end(Some(ButtonId::Pen(2))),
        ToolbarEvent::Dragged
    );

    // Releasing over another button is not a click.
    interaction.begin(Some(ButtonId::Pen(2)), press, origin, true);
    assert_eq!(interaction.end(Some(ButtonId::Pen(3))), ToolbarEvent::None);

    // Dragging is blocked while the toolbar is animating (legacy requires
    // ButtonsEntering == 0), but the click still registers.
    interaction.begin(Some(ButtonId::Pen(2)), press, origin, false);
    assert_eq!(
        interaction.drag_to(PhysicalPoint::new(1400, 900), bounds, panel),
        None
    );
    assert_eq!(
        interaction.end(Some(ButtonId::Pen(2))),
        ToolbarEvent::Click(ButtonId::Pen(2))
    );

    // The dragged origin is clamped to the visible screen bounds.
    interaction.begin(None, press, PhysicalPoint::new(10, 10), true);
    assert_eq!(
        interaction.drag_to(PhysicalPoint::new(0, 0), bounds, panel),
        Some(PhysicalPoint::new(0, 0))
    );
    interaction.begin(None, press, PhysicalPoint::new(2400, 1500), true);
    assert_eq!(
        interaction.drag_to(PhysicalPoint::new(3000, 2000), bounds, panel),
        Some(PhysicalPoint::new(1560, 1504)),
        "panel (1000x96) must stay inside 2560x1600"
    );
}

#[test]
fn button_codes_roundtrip_for_the_click_notification() {
    use ButtonId::*;
    let buttons = [
        Dock,
        Eraser,
        Pan,
        Pointer,
        PenWidth,
        InkVisible,
        Snapshot,
        Undo,
        Clear,
        Exit,
        Pen(0),
        Pen(5),
        Pen(9),
    ];
    for button in buttons {
        assert_eq!(
            ButtonId::from_code(button.code()),
            Some(button),
            "{button:?} must survive the notification encoding"
        );
    }
    assert_eq!(ButtonId::from_code(0), None);
    assert_eq!(
        ButtonId::from_code(0x100 + 10),
        None,
        "only ten pen slots exist"
    );
    assert_eq!(ButtonId::from_code(11), None);
}

#[test]
fn assets_resolve_next_to_the_executable_and_select_tray_colour() {
    let executable = std::path::Path::new(r"D:\portable\plnk-v0.1.0\plnk.exe");
    assert_eq!(
        icon_directory_for(executable),
        std::path::PathBuf::from(r"D:\portable\plnk-v0.1.0\assets\icons")
    );
    assert_eq!(tray_icon_filename(false), "icon_red.ico");
    assert_eq!(tray_icon_filename(true), "icon_white.ico");
    assert_eq!(plnk::tray::TRAY_TOOLTIP, "plnk 0.1.0");
}

#[test]
fn every_toolbar_state_maps_to_an_original_legacy_png() {
    let expected = [
        (Icon::Dock, "dock.png"),
        (Icon::DockBack, "dockback.png"),
        (Icon::Pencil, "pencil1.png"),
        (Icon::PencilActive, "pencil_act.png"),
        (Icon::Highlighter, "highlighter1.png"),
        (Icon::HighlighterActive, "highlighter_act.png"),
        (Icon::Eraser, "eraser.png"),
        (Icon::EraserActive, "eraseract.png"),
        (Icon::Pointer, "pointer1.png"),
        (Icon::PointerActive, "pointer_act.png"),
        (Icon::Pan, "pan.png"),
        (Icon::PanActive, "pan_act.png"),
        (Icon::PenWidth, "penwidth.png"),
        (Icon::InkVisible, "visible.png"),
        (Icon::InkHidden, "visible_not.png"),
        (Icon::Snapshot, "snap.png"),
        (Icon::Undo, "undo.png"),
        (Icon::Clear, "garbage.png"),
        (Icon::Exit, "exit.png"),
    ];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
    for (icon, filename) in expected {
        assert_eq!(
            icon.asset_name(),
            filename,
            "wrong legacy file for {icon:?}"
        );
        let bytes = std::fs::read(root.join(filename))
            .unwrap_or_else(|error| panic!("missing {filename}: {error}"));
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "{filename} is not a PNG");
        let width = u32::from_be_bytes(bytes[16..20].try_into().expect("PNG width"));
        let height = u32::from_be_bytes(bytes[20..24].try_into().expect("PNG height"));
        if matches!(icon, Icon::Dock | Icon::DockBack) {
            assert_eq!((width, height), (96, 128), "legacy dock bitmap size");
        } else {
            assert_eq!((width, height), (128, 128), "legacy square bitmap size");
        }
    }
}
