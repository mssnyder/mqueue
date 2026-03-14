//! Compose window for writing and sending emails.
//!
//! Supports new messages, replies, reply-all, and forwarding.
//! Address fields (To/Cc/Bcc) have autocomplete from synced contacts.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::gdk;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;
use webkit6::prelude::*;

/// A contact for autocomplete: (display_name, email).
#[derive(Debug, Clone)]
pub struct ContactEntry {
    pub display_name: Option<String>,
    pub email: String,
}

/// The kind of compose action.
#[derive(Debug, Clone, Default)]
pub enum ComposeMode {
    #[default]
    New,
    Reply {
        from: String,
        subject: String,
        date: String,
        body: String,
        message_id: Option<String>,
        references: Option<String>,
    },
    ReplyAll {
        from: String,
        to: Vec<String>,
        cc: Vec<String>,
        subject: String,
        date: String,
        body: String,
        message_id: Option<String>,
        references: Option<String>,
    },
    Forward {
        from: String,
        subject: String,
        date: String,
        to: String,
        body: String,
    },
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MqComposeWindow {
        pub from_dropdown: RefCell<Option<gtk::DropDown>>,
        pub to_entry: RefCell<Option<gtk::Entry>>,
        pub cc_entry: RefCell<Option<gtk::Entry>>,
        pub bcc_entry: RefCell<Option<gtk::Entry>>,
        pub subject_entry: RefCell<Option<gtk::Entry>>,
        pub body_view: RefCell<Option<gtk::TextView>>,
        pub send_button: RefCell<Option<gtk::Button>>,
        pub cc_row: RefCell<Option<gtk::Box>>,
        pub bcc_row: RefCell<Option<gtk::Box>>,
        pub sending_bar: RefCell<Option<gtk::ProgressBar>>,
        pub form_box: RefCell<Option<gtk::Box>>,
        pub body_scrolled: RefCell<Option<gtk::ScrolledWindow>>,
        pub attachments_box: RefCell<Option<gtk::Box>>,
        pub accounts: RefCell<Vec<(i64, String)>>,
        pub contacts: Rc<RefCell<Vec<ContactEntry>>>,
        pub attachments: Rc<RefCell<Vec<(String, std::path::PathBuf)>>>,
        /// Set to true after a successful send so close_request skips the dialog.
        pub sent_successfully: std::cell::Cell<bool>,
        /// Draft ID if this compose is backed by a saved draft.
        pub draft_id: RefCell<Option<i64>>,
        /// Callback to save the compose content as a draft.
        pub save_draft_callback: RefCell<Option<Box<dyn Fn(&super::MqComposeWindow)>>>,
        /// Cached body HTML from the rich text editor WebView.
        pub cached_body_html: std::rc::Rc<RefCell<String>>,
        /// The rich text editor WebView (replaces body_view when active).
        pub body_webview: std::rc::Rc<RefCell<Option<webkit6::WebView>>>,
        /// Format toolbar buttons keyed by command name, for state highlighting.
        pub fmt_buttons: Rc<RefCell<Vec<(String, gtk::Button)>>>,
    }

    impl std::fmt::Debug for MqComposeWindow {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("MqComposeWindow").finish_non_exhaustive()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MqComposeWindow {
        const NAME: &'static str = "MqComposeWindow";
        type Type = super::MqComposeWindow;
        type ParentType = adw::Window;
    }

    impl ObjectImpl for MqComposeWindow {
        fn constructed(&self) {
            self.parent_constructed();

            let window = self.obj();
            window.set_title(Some("New Message"));
            window.set_default_size(660, 500);
            window.set_size_request(360, 350);

            let main_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .build();

            // Header bar
            let header = adw::HeaderBar::new();

            let send_button = gtk::Button::builder()
                .label("Send")
                .css_classes(["suggested-action"])
                .build();
            header.pack_end(&send_button);

            // Cc/Bcc toggle button
            let cc_toggle = gtk::ToggleButton::builder()
                .label("Cc/Bcc")
                .build();
            header.pack_end(&cc_toggle);

            // Attach button
            let attach_button = gtk::Button::builder()
                .icon_name("mail-attachment-symbolic")
                .tooltip_text("Attach file")
                .build();
            header.pack_end(&attach_button);

            main_box.append(&header);

            // Sending progress bar (hidden by default)
            let sending_bar = gtk::ProgressBar::builder()
                .pulse_step(0.1)
                .visible(false)
                .build();
            sending_bar.add_css_class("osd");
            main_box.append(&sending_bar);

            // Form fields
            let form = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(0)
                .margin_start(12)
                .margin_end(12)
                .margin_top(8)
                .build();

            // From
            let from_row = Self::make_field_row("From:");
            let from_dropdown = gtk::DropDown::from_strings(&[]);
            from_dropdown.set_hexpand(true);
            from_row.append(&from_dropdown);
            form.append(&from_row);

            // To
            let to_row = Self::make_field_row("To:");
            let to_entry = gtk::Entry::builder()
                .hexpand(true)
                .placeholder_text("recipient@example.com")
                .build();
            to_row.append(&to_entry);
            form.append(&to_row);

            // Cc (hidden by default)
            let cc_row = Self::make_field_row("Cc:");
            let cc_entry = gtk::Entry::builder()
                .hexpand(true)
                .placeholder_text("cc@example.com")
                .build();
            cc_row.append(&cc_entry);
            cc_row.set_visible(false);
            form.append(&cc_row);

            // Bcc (hidden by default)
            let bcc_row = Self::make_field_row("Bcc:");
            let bcc_entry = gtk::Entry::builder()
                .hexpand(true)
                .placeholder_text("bcc@example.com")
                .build();
            bcc_row.append(&bcc_entry);
            bcc_row.set_visible(false);
            form.append(&bcc_row);

            // Subject
            let subject_row = Self::make_field_row("Subject:");
            let subject_entry = gtk::Entry::builder()
                .hexpand(true)
                .build();
            subject_row.append(&subject_entry);
            form.append(&subject_row);

            main_box.append(&form);

            // Attachments display area (hidden until files added)
            let attachments_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(2)
                .margin_start(12)
                .margin_end(12)
                .margin_top(4)
                .margin_bottom(4)
                .visible(false)
                .build();
            main_box.append(&attachments_box);

            // Separator
            main_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

            // Formatting toolbar
            let toolbar = Self::create_formatting_toolbar(&self);
            main_box.append(&toolbar);
            main_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

            // Body rich text editor (WebView with contenteditable)
            let body_webview = webkit6::WebView::new();
            if let Some(settings) = WebViewExt::settings(&body_webview) {
                settings.set_enable_javascript(true);
                settings.set_enable_javascript_markup(true);
                settings.set_enable_developer_extras(false);
                settings.set_default_font_size(16);
                settings.set_default_monospace_font_size(14);
            }

            // Transparent background
            let transparent = gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0);
            body_webview.set_background_color(&transparent);
            body_webview.set_vexpand(true);
            body_webview.set_hexpand(true);

            // Register a single script message handler for all editor updates.
            // Messages prefixed with "html:" carry body content; "fmt:" carry
            // formatting state for toolbar button highlighting.
            let ucm = body_webview.user_content_manager().unwrap();
            ucm.register_script_message_handler("mqUpdate", None);
            let cached_html = std::rc::Rc::clone(&self.cached_body_html);
            let fmt_btns = Rc::clone(&self.fmt_buttons);
            ucm.connect_script_message_received(None, move |_ucm, value| {
                let msg: String = format!("{}", value);
                let msg = msg.trim_matches('"');
                if let Some(html) = msg.strip_prefix("html:") {
                    *cached_html.borrow_mut() = html.to_string();
                } else if let Some(state) = msg.strip_prefix("fmt:") {
                    let parts: Vec<&str> = state.split(',').collect();
                    if parts.len() < 5 { return; }
                    let block = parts[4];

                    for (key, btn) in fmt_btns.borrow().iter() {
                        let active = match key.as_str() {
                            "bold" => parts[0] == "bold",
                            "italic" => parts[1] == "italic",
                            "underline" => parts[2] == "underline",
                            "strikeThrough" => parts[3] == "strikeThrough",
                            "h1" | "h2" | "h3" | "pre" => block == key,
                            _ => false,
                        };
                        if active {
                            btn.add_css_class("fmt-active");
                        } else {
                            btn.remove_css_class("fmt-active");
                        }
                    }
                }
            });

            // Detect dark mode for editor styling
            let is_dark = adw::StyleManager::default().is_dark();
            let (text_color, placeholder_color, code_bg) = if is_dark {
                ("#e0e0e0", "#888", "rgba(255,255,255,0.08)")
            } else {
                ("#1a1a1a", "#999", "rgba(0,0,0,0.05)")
            };

            let editor_html = format!(
                r#"<!DOCTYPE html><html><head><meta charset="utf-8"><style>
body {{
    font-family: system-ui, -apple-system, sans-serif;
    font-size: 16px;
    margin: 0;
    padding: 12px;
    min-height: 100%;
    outline: none;
    color: {text_color};
    background: transparent;
    line-height: 1.5;
    word-wrap: break-word;
    overflow-wrap: break-word;
}}
body:empty:before {{
    content: 'Compose your message...';
    color: {placeholder_color};
    pointer-events: none;
}}
pre, code {{
    background: {code_bg};
    border-radius: 3px;
    padding: 2px 4px;
    font-family: monospace;
}}
pre {{
    padding: 8px 12px;
}}
blockquote {{
    border-left: 3px solid #ccc;
    margin-left: 0;
    padding-left: 12px;
    color: #666;
}}
h1 {{ font-size: 1.5em; margin: 0.4em 0; }}
h2 {{ font-size: 1.3em; margin: 0.4em 0; }}
h3 {{ font-size: 1.1em; margin: 0.4em 0; }}
a {{ color: #1a73e8; }}
ul, ol {{ padding-left: 24px; }}
</style>
<script>
document.addEventListener('DOMContentLoaded', function() {{
    var mq = window.webkit.messageHandlers.mqUpdate;
    function sendFormatState() {{
        var block = document.queryCommandValue('formatBlock');
        mq.postMessage('fmt:' + [
            document.queryCommandState('bold') ? 'bold' : '',
            document.queryCommandState('italic') ? 'italic' : '',
            document.queryCommandState('underline') ? 'underline' : '',
            document.queryCommandState('strikeThrough') ? 'strikeThrough' : '',
            block
        ].join(','));
    }}
    document.body.addEventListener('input', function() {{
        mq.postMessage('html:' + document.body.innerHTML);
        sendFormatState();
    }});
    document.addEventListener('selectionchange', function() {{
        var sel = window.getSelection();
        if (sel.rangeCount > 0 && document.body.contains(sel.anchorNode)) {{
            window._savedRange = sel.getRangeAt(0).cloneRange();
        }}
        sendFormatState();
    }});
}});
function mqRestoreSelection() {{
    document.body.focus();
    if (window._savedRange) {{
        var sel = window.getSelection();
        sel.removeAllRanges();
        sel.addRange(window._savedRange);
    }}
}}
</script>
</head>
<body contenteditable="true" spellcheck="true"></body>
</html>"#
            );

            body_webview.load_html(&editor_html, None);

            // Keep a plain text fallback view hidden (for backward compat)
            let body_view = gtk::TextView::builder()
                .wrap_mode(gtk::WrapMode::WordChar)
                .top_margin(12)
                .bottom_margin(12)
                .left_margin(12)
                .right_margin(12)
                .visible(false)
                .build();

            let scrolled = gtk::ScrolledWindow::builder()
                .vexpand(true)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .visible(false)
                .build();
            scrolled.set_child(Some(&body_view));

            main_box.append(&body_webview);
            main_box.append(&scrolled);

            window.set_content(Some(&main_box));

            // Wire Cc/Bcc toggle
            let cc_row_clone = cc_row.clone();
            let bcc_row_clone = bcc_row.clone();
            cc_toggle.connect_toggled(move |btn| {
                let show = btn.is_active();
                cc_row_clone.set_visible(show);
                bcc_row_clone.set_visible(show);
            });

            // Escape closes the compose window (triggers draft save dialog if content)
            // Ctrl+Return sends the message
            let key_controller = gtk::EventControllerKey::new();
            let key_win = window.clone();
            let key_send = send_button.clone();
            key_controller.connect_key_pressed(move |_, key, _, modifiers| {
                if key == gdk::Key::Escape {
                    key_win.close();
                    glib::Propagation::Stop
                } else if key == gdk::Key::Return
                    && modifiers.contains(gdk::ModifierType::CONTROL_MASK)
                {
                    key_send.emit_clicked();
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
            window.add_controller(key_controller);

            // Wire attach button
            {
                let window_ref = window.clone();
                let att_box = attachments_box.clone();
                let att_list = self.attachments.clone();
                attach_button.connect_clicked(move |_| {
                    let dialog = gtk::FileDialog::builder()
                        .title("Attach File")
                        .build();
                    let att_box = att_box.clone();
                    let att_list = att_list.clone();
                    let win_ref = window_ref.clone();
                    dialog.open_multiple(
                        Some(&win_ref.clone().upcast::<gtk::Window>()),
                        Option::<&gtk::gio::Cancellable>::None,
                        move |result: Result<gtk::gio::ListModel, glib::Error>| {
                            if let Ok(files) = result {
                                for i in 0..files.n_items() {
                                    if let Some(file) = files.item(i).and_then(|o| o.downcast::<gtk::gio::File>().ok()) {
                                        if let Some(path) = file.path() {
                                            // Check attachment size limit (25 MB total)
                                            let file_size = std::fs::metadata(&path)
                                                .map(|m| m.len())
                                                .unwrap_or(0);
                                            let current_total: u64 = att_list.borrow().iter()
                                                .map(|(_, p): &(String, std::path::PathBuf)| {
                                                    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
                                                })
                                                .sum();
                                            if current_total + file_size > 25 * 1024 * 1024 {
                                                let dialog = adw::AlertDialog::builder()
                                                    .heading("File too large")
                                                    .body("Total attachment size exceeds the 25 MB limit.")
                                                    .build();
                                                dialog.add_response("ok", "OK");
                                                dialog.present(Some(&win_ref.clone().upcast::<gtk::Window>()));
                                                continue;
                                            }
                                            let filename = path.file_name()
                                                .map(|n: &std::ffi::OsStr| n.to_string_lossy().to_string())
                                                .unwrap_or_else(|| "file".to_string());
                                            let path_for_remove = path.clone();
                                            att_list.borrow_mut().push((filename.clone(), path));

                                            // Add row to UI
                                            let row = gtk::Box::builder()
                                                .orientation(gtk::Orientation::Horizontal)
                                                .spacing(8)
                                                .build();
                                            let icon = gtk::Image::builder()
                                                .icon_name("mail-attachment-symbolic")
                                                .build();
                                            row.append(&icon);
                                            let label = gtk::Label::builder()
                                                .label(&filename)
                                                .xalign(0.0)
                                                .hexpand(true)
                                                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                                                .build();
                                            row.append(&label);

                                            let remove_btn = gtk::Button::builder()
                                                .icon_name("window-close-symbolic")
                                                .css_classes(["flat", "circular"])
                                                .tooltip_text("Remove")
                                                .build();
                                            let att_box2 = att_box.clone();
                                            let att_list2 = att_list.clone();
                                            let att_path = path_for_remove;
                                            let row_ref = row.clone();
                                            remove_btn.connect_clicked(move |_| {
                                                att_box2.remove(&row_ref);
                                                // Remove by path (unique) rather than filename (may have duplicates)
                                                att_list2.borrow_mut().retain(|(_, p)| p != &att_path);
                                                if att_list2.borrow().is_empty() {
                                                    att_box2.set_visible(false);
                                                }
                                            });
                                            row.append(&remove_btn);
                                            att_box.append(&row);
                                            att_box.set_visible(true);
                                        }
                                    }
                                }
                            }
                        },
                    );
                });
            }

            // Set up autocomplete on address fields
            let contacts = self.contacts.clone();
            setup_address_autocomplete(&to_entry, contacts.clone());
            setup_address_autocomplete(&cc_entry, contacts.clone());
            setup_address_autocomplete(&bcc_entry, contacts);

            // Store references
            *self.from_dropdown.borrow_mut() = Some(from_dropdown);
            *self.to_entry.borrow_mut() = Some(to_entry);
            *self.cc_entry.borrow_mut() = Some(cc_entry);
            *self.bcc_entry.borrow_mut() = Some(bcc_entry);
            *self.subject_entry.borrow_mut() = Some(subject_entry);
            *self.body_view.borrow_mut() = Some(body_view);
            *self.body_webview.borrow_mut() = Some(body_webview);
            *self.send_button.borrow_mut() = Some(send_button);
            *self.cc_row.borrow_mut() = Some(cc_row);
            *self.bcc_row.borrow_mut() = Some(bcc_row);
            *self.sending_bar.borrow_mut() = Some(sending_bar);
            *self.form_box.borrow_mut() = Some(form);
            *self.body_scrolled.borrow_mut() = Some(scrolled);
            *self.attachments_box.borrow_mut() = Some(attachments_box);
        }
    }

    impl WidgetImpl for MqComposeWindow {}
    impl WindowImpl for MqComposeWindow {
        fn close_request(&self) -> glib::Propagation {
            // If sent successfully or no content, close without prompting
            if self.sent_successfully.get() {
                return self.parent_close_request();
            }

            let window = self.obj();
            if !window.has_content() {
                return self.parent_close_request();
            }

            // Show save/discard confirmation dialog
            let win = window.clone();
            let has_save = self.save_draft_callback.borrow().is_some();
            let dialog = adw::AlertDialog::builder()
                .heading("Save draft?")
                .body("Your message has not been sent.")
                .close_response("cancel")
                .default_response(if has_save { "save" } else { "cancel" })
                .build();
            dialog.add_response("cancel", "Cancel");
            if has_save {
                dialog.add_response("save", "Save Draft");
                dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
            }
            dialog.add_response("discard", "Discard");
            dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);

            dialog.connect_response(None, move |_, response| {
                match response {
                    "save" => {
                        if let Some(ref cb) = *win.imp().save_draft_callback.borrow() {
                            cb(&win);
                        }
                        win.imp().sent_successfully.set(true);
                        win.close();
                    }
                    "discard" => {
                        win.imp().sent_successfully.set(true);
                        win.close();
                    }
                    _ => {} // cancel — do nothing
                }
            });

            dialog.present(Some(&*window));

            // Inhibit the default close — the dialog callback will close if confirmed
            glib::Propagation::Stop
        }
    }
    impl AdwWindowImpl for MqComposeWindow {}

    impl MqComposeWindow {
        /// Create the rich text formatting toolbar.
        fn create_formatting_toolbar(&self) -> gtk::Box {
            let toolbar = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(2)
                .margin_start(8)
                .margin_end(8)
                .margin_top(4)
                .margin_bottom(4)
                .css_classes(["compose-toolbar"])
                .build();

            let wv_ref = &self.body_webview;  // Rc<RefCell<...>> — cloning gives shared ref

            // Helper to create a simple formatting button.
            // Buttons are non-focusable so they don't steal focus (and thus
            // the text selection) from the WebView when clicked.
            // Helper to create a formatting button that executes a command on
            // the WebView without stealing focus.  We intercept the click in
            // the CAPTURE phase — before the button receives it — so the
            // WebView retains focus and the text selection stays intact.
            // JS snippet that runs a formatting command and immediately
            // reports the new state back so the toolbar buttons update.
            let fmt_js = |cmd: &str| -> String {
                format!(
                    "document.execCommand('{}', false, null); \
                     (function() {{ \
                         var block = document.queryCommandValue('formatBlock'); \
                         var s = 'fmt:' + [ \
                             document.queryCommandState('bold') ? 'bold' : '', \
                             document.queryCommandState('italic') ? 'italic' : '', \
                             document.queryCommandState('underline') ? 'underline' : '', \
                             document.queryCommandState('strikeThrough') ? 'strikeThrough' : '', \
                             block \
                         ].join(','); \
                         window.webkit.messageHandlers.mqUpdate.postMessage(s); \
                     }})();",
                    cmd
                )
            };

            let fmt_btn_list = &self.fmt_buttons;

            let make_fmt_btn = |icon: &str, tooltip: &str, command: &str| -> gtk::Button {
                let btn = gtk::Button::builder()
                    .icon_name(icon)
                    .tooltip_text(tooltip)
                    .focusable(false)
                    .css_classes(["flat", "circular"])
                    .build();
                let js = fmt_js(command);
                let wv = wv_ref.clone();
                let gesture = gtk::GestureClick::new();
                gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
                gesture.connect_pressed(move |g, _, _, _| {
                    if let Some(ref wv) = *wv.borrow() {
                        wv.evaluate_javascript(&js, None, None, None::<&gtk::gio::Cancellable>, |_| {});
                    }
                    g.set_state(gtk::EventSequenceState::Claimed);
                });
                btn.add_controller(gesture);
                // Register for format state highlighting
                fmt_btn_list.borrow_mut().push((command.to_string(), btn.clone()));
                btn
            };

            // Text formatting
            toolbar.append(&make_fmt_btn("format-text-bold-symbolic", "Bold (Ctrl+B)", "bold"));
            toolbar.append(&make_fmt_btn("format-text-italic-symbolic", "Italic (Ctrl+I)", "italic"));
            toolbar.append(&make_fmt_btn("format-text-underline-symbolic", "Underline (Ctrl+U)", "underline"));
            toolbar.append(&make_fmt_btn("format-text-strikethrough-symbolic", "Strikethrough", "strikeThrough"));

            // Separator
            let sep1 = gtk::Separator::new(gtk::Orientation::Vertical);
            sep1.set_margin_start(4);
            sep1.set_margin_end(4);
            toolbar.append(&sep1);

            // Headings — toggle: click again to revert to normal paragraph
            for (label, tag) in [("H1", "h1"), ("H2", "h2"), ("H3", "h3")] {
                let btn = gtk::Button::builder()
                    .label(label)
                    .tooltip_text(&format!("Heading {}", &label[1..]))
                    .focusable(false)
                    .css_classes(["flat", "circular"])
                    .build();
                let wv = wv_ref.clone();
                let tag_s = tag.to_string();
                let gesture = gtk::GestureClick::new();
                gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
                let state_js = fmt_js("noop").replace("document.execCommand('noop', false, null); ", "");
                gesture.connect_pressed(move |g, _, _, _| {
                    if let Some(ref wv) = *wv.borrow() {
                        let js = format!(
                            "var cur = document.queryCommandValue('formatBlock'); \
                             document.execCommand('formatBlock', false, cur === '{}' ? 'p' : '{}'); {}",
                            tag_s, tag_s, state_js
                        );
                        wv.evaluate_javascript(&js, None, None, None::<&gtk::gio::Cancellable>, |_| {});
                    }
                    g.set_state(gtk::EventSequenceState::Claimed);
                });
                btn.add_controller(gesture);
                fmt_btn_list.borrow_mut().push((tag.to_string(), btn.clone()));
                toolbar.append(&btn);
            }

            // Normal text button
            let normal_btn = gtk::Button::builder()
                .label("P")
                .tooltip_text("Normal text")
                .focusable(false)
                .css_classes(["flat", "circular"])
                .build();
            {
                let wv = wv_ref.clone();
                let gesture = gtk::GestureClick::new();
                gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
                gesture.connect_pressed(move |g, _, _, _| {
                    if let Some(ref wv) = *wv.borrow() {
                        wv.evaluate_javascript(
                            "document.execCommand('formatBlock', false, 'p');",
                            None, None, None::<&gtk::gio::Cancellable>, |_| {},
                        );
                    }
                    g.set_state(gtk::EventSequenceState::Claimed);
                });
                normal_btn.add_controller(gesture);
            }
            toolbar.append(&normal_btn);

            let sep2 = gtk::Separator::new(gtk::Orientation::Vertical);
            sep2.set_margin_start(4);
            sep2.set_margin_end(4);
            toolbar.append(&sep2);

            // Lists
            toolbar.append(&make_fmt_btn("view-list-symbolic", "Bullet list", "insertUnorderedList"));
            toolbar.append(&make_fmt_btn("view-list-ordered-symbolic", "Numbered list", "insertOrderedList"));

            let sep3 = gtk::Separator::new(gtk::Orientation::Vertical);
            sep3.set_margin_start(4);
            sep3.set_margin_end(4);
            toolbar.append(&sep3);

            // Code block — toggle: click again to revert to <p>
            {
                let btn = gtk::Button::builder()
                    .icon_name("utilities-terminal-symbolic")
                    .tooltip_text("Code block")
                    .focusable(false)
                    .css_classes(["flat", "circular"])
                    .build();
                let wv = wv_ref.clone();
                let gesture = gtk::GestureClick::new();
                gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
                let state_js = fmt_js("noop").replace("document.execCommand('noop', false, null); ", "");
                gesture.connect_pressed(move |g, _, _, _| {
                    if let Some(ref wv) = *wv.borrow() {
                        let js = format!(
                            "var cur = document.queryCommandValue('formatBlock'); \
                             document.execCommand('formatBlock', false, cur === 'pre' ? 'p' : 'pre'); {}",
                            state_js
                        );
                        wv.evaluate_javascript(&js, None, None, None::<&gtk::gio::Cancellable>, |_| {});
                    }
                    g.set_state(gtk::EventSequenceState::Claimed);
                });
                btn.add_controller(gesture);
                fmt_btn_list.borrow_mut().push(("pre".to_string(), btn.clone()));
                toolbar.append(&btn);
            }

            // Link
            {
                let link_btn = gtk::Button::builder()
                    .icon_name("insert-link-symbolic")
                    .tooltip_text("Insert link")
                    .focusable(false)
                    .css_classes(["flat", "circular"])
                    .build();
                let wv = wv_ref.clone();
                let window = self.obj().clone();
                link_btn.connect_clicked(move |_| {
                    let wv = wv.clone();
                    let dialog = adw::AlertDialog::builder()
                        .heading("Insert Link")
                        .body("Enter URL:")
                        .build();
                    let entry = gtk::Entry::builder()
                        .placeholder_text("https://example.com")
                        .build();
                    dialog.set_extra_child(Some(&entry));
                    dialog.add_responses(&[("cancel", "Cancel"), ("insert", "Insert")]);
                    dialog.set_response_appearance("insert", adw::ResponseAppearance::Suggested);
                    dialog.set_default_response(Some("insert"));
                    let entry_ref = entry.clone();
                    dialog.connect_response(None, move |_, response| {
                        if response == "insert" {
                            let url = entry_ref.text().to_string();
                            if !url.is_empty() {
                                if let Some(ref wv) = *wv.borrow() {
                                    let url_escaped = url.replace('\'', "\\'");
                                    let js = format!("document.execCommand('createLink', false, '{}');", url_escaped);
                                    wv.evaluate_javascript(&js, None, None, None::<&gtk::gio::Cancellable>, |_| {});
                                }
                            }
                        }
                    });
                    dialog.present(Some(&window.clone().upcast::<gtk::Window>()));
                });
                toolbar.append(&link_btn);
            }

            let sep4 = gtk::Separator::new(gtk::Orientation::Vertical);
            sep4.set_margin_start(4);
            sep4.set_margin_end(4);
            toolbar.append(&sep4);

            // Text color
            {
                let color_btn = gtk::Button::builder()
                    .icon_name("color-select-symbolic")
                    .tooltip_text("Text color")
                    .focusable(false)
                    .css_classes(["flat", "circular"])
                    .build();
                let wv = wv_ref.clone();
                let window = self.obj().clone();
                color_btn.connect_clicked(move |_| {
                    let wv = wv.clone();
                    let dialog = gtk::ColorDialog::builder()
                        .title("Text Color")
                        .build();
                    let win = window.clone();
                    dialog.choose_rgba(
                        Some(&win.clone().upcast::<gtk::Window>()),
                        None,
                        None::<&gtk::gio::Cancellable>,
                        move |result| {
                            if let Ok(color) = result {
                                let hex = format!(
                                    "#{:02x}{:02x}{:02x}",
                                    (color.red() * 255.0) as u8,
                                    (color.green() * 255.0) as u8,
                                    (color.blue() * 255.0) as u8,
                                );
                                if let Some(ref wv) = *wv.borrow() {
                                    let js = format!("document.execCommand('foreColor', false, '{}');", hex);
                                    wv.evaluate_javascript(&js, None, None, None::<&gtk::gio::Cancellable>, |_| {});
                                }
                            }
                        },
                    );
                });
                toolbar.append(&color_btn);
            }

            // Highlight color
            {
                let highlight_btn = gtk::Button::builder()
                    .icon_name("document-edit-symbolic")
                    .tooltip_text("Highlight color")
                    .focusable(false)
                    .css_classes(["flat", "circular"])
                    .build();
                let wv = wv_ref.clone();
                let window = self.obj().clone();
                highlight_btn.connect_clicked(move |_| {
                    let wv = wv.clone();
                    let dialog = gtk::ColorDialog::builder()
                        .title("Highlight Color")
                        .build();
                    let win = window.clone();
                    dialog.choose_rgba(
                        Some(&win.clone().upcast::<gtk::Window>()),
                        None,
                        None::<&gtk::gio::Cancellable>,
                        move |result| {
                            if let Ok(color) = result {
                                let hex = format!(
                                    "#{:02x}{:02x}{:02x}",
                                    (color.red() * 255.0) as u8,
                                    (color.green() * 255.0) as u8,
                                    (color.blue() * 255.0) as u8,
                                );
                                if let Some(ref wv) = *wv.borrow() {
                                    let js = format!("document.execCommand('hiliteColor', false, '{}');", hex);
                                    wv.evaluate_javascript(&js, None, None, None::<&gtk::gio::Cancellable>, |_| {});
                                }
                            }
                        },
                    );
                });
                toolbar.append(&highlight_btn);
            }

            // Font size dropdown
            {
                let size_btn = gtk::MenuButton::builder()
                    .icon_name("font-x-generic-symbolic")
                    .tooltip_text("Font size")
                    .css_classes(["flat", "circular"])
                    .build();
                let popover = gtk::Popover::new();
                let size_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
                for (label, size) in [("Small", "2"), ("Normal", "3"), ("Large", "5"), ("Huge", "7")] {
                    let btn = gtk::Button::builder()
                        .label(label)
                        .css_classes(["flat"])
                        .build();
                    let wv = wv_ref.clone();
                    let pop = popover.clone();
                    let sz = size.to_string();
                    btn.connect_clicked(move |_| {
                        if let Some(ref wv) = *wv.borrow() {
                            let js = format!("document.execCommand('fontSize', false, '{}');", sz);
                            wv.evaluate_javascript(&js, None, None, None::<&gtk::gio::Cancellable>, |_| {});
                        }
                        pop.popdown();
                    });
                    size_box.append(&btn);
                }
                popover.set_child(Some(&size_box));
                size_btn.set_popover(Some(&popover));
                toolbar.append(&size_btn);
            }

            let sep5 = gtk::Separator::new(gtk::Orientation::Vertical);
            sep5.set_margin_start(4);
            sep5.set_margin_end(4);
            toolbar.append(&sep5);

            // Emoji picker
            {
                let emoji_btn = gtk::MenuButton::builder()
                    .icon_name("face-smile-symbolic")
                    .tooltip_text("Insert emoji")
                    .css_classes(["flat", "circular"])
                    .build();
                let emoji_chooser = gtk::EmojiChooser::new();
                let wv = wv_ref.clone();
                emoji_chooser.connect_emoji_picked(move |_, emoji| {
                    if let Some(ref wv) = *wv.borrow() {
                        let emoji_escaped = emoji.replace('\'', "\\'");
                        let js = format!("document.execCommand('insertText', false, '{}');", emoji_escaped);
                        wv.evaluate_javascript(&js, None, None, None::<&gtk::gio::Cancellable>, |_| {});
                    }
                });
                emoji_btn.set_popover(Some(&emoji_chooser));
                toolbar.append(&emoji_btn);
            }

            // Wrap in a ScrolledWindow so toolbar items don't get truncated on narrow windows
            let toolbar_scroll = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .vscrollbar_policy(gtk::PolicyType::Never)
                .build();
            toolbar_scroll.set_child(Some(&toolbar));

            let wrapper = gtk::Box::new(gtk::Orientation::Vertical, 0);
            wrapper.append(&toolbar_scroll);
            wrapper
        }

        fn make_field_row(label_text: &str) -> gtk::Box {
            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .margin_top(4)
                .margin_bottom(4)
                .build();
            let label = gtk::Label::builder()
                .label(label_text)
                .width_chars(8)
                .xalign(1.0)
                .css_classes(["dim-label"])
                .build();
            row.append(&label);
            row
        }
    }
}

/// Set up autocomplete on an address Entry using a Popover + ListBox.
///
/// Filters contacts as the user types, matching the token after the last comma.
/// On selection, appends the chosen address to the entry.
fn setup_address_autocomplete(entry: &gtk::Entry, contacts: Rc<RefCell<Vec<ContactEntry>>>) {
    let popover = gtk::Popover::builder()
        .autohide(false)
        .has_arrow(false)
        .build();

    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .max_content_height(200)
        .propagate_natural_height(true)
        .build();

    let listbox = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .build();
    listbox.add_css_class("boxed-list");

    scrolled.set_child(Some(&listbox));
    popover.set_child(Some(&scrolled));
    popover.set_parent(entry);

    // When a row is activated, insert the contact into the entry
    let entry_for_activate = entry.clone();
    let popover_for_activate = popover.clone();
    listbox.connect_row_activated(move |_, row| {
        if let Some(email) = row.widget_name().strip_prefix("contact:") {
            let current = entry_for_activate.text().to_string();
            // Replace the current incomplete token with the selected address
            let new_text = if let Some(last_comma) = current.rfind(',') {
                format!("{}, {email}, ", &current[..=last_comma])
            } else {
                format!("{email}, ")
            };
            entry_for_activate.set_text(&new_text);
            entry_for_activate.set_position(-1); // cursor to end
        }
        popover_for_activate.popdown();
    });

    // Filter and show popover on text changes
    let popover_for_changed = popover.clone();
    let listbox_for_changed = listbox.clone();
    entry.connect_changed(move |entry| {
        let text = entry.text().to_string();

        // Extract the current token being typed (after last comma)
        let current_token = if let Some(last_comma) = text.rfind(',') {
            text[last_comma + 1..].trim()
        } else {
            text.trim()
        };

        if current_token.len() < 2 {
            popover_for_changed.popdown();
            return;
        }

        let query = current_token.to_lowercase();
        let contacts = contacts.borrow();

        // Filter matching contacts (max 8 results)
        let matches: Vec<&ContactEntry> = contacts
            .iter()
            .filter(|c| {
                c.email.to_lowercase().contains(&query)
                    || c.display_name
                        .as_ref()
                        .map(|n| n.to_lowercase().contains(&query))
                        .unwrap_or(false)
            })
            .take(8)
            .collect();

        if matches.is_empty() {
            popover_for_changed.popdown();
            return;
        }

        // Clear old rows
        while let Some(child) = listbox_for_changed.first_child() {
            listbox_for_changed.remove(&child);
        }

        // Add matching contacts
        for contact in &matches {
            let label_text = if let Some(ref name) = contact.display_name {
                format!("{name} <{}>", contact.email)
            } else {
                contact.email.clone()
            };

            let label = gtk::Label::builder()
                .label(&label_text)
                .xalign(0.0)
                .margin_start(8)
                .margin_end(8)
                .margin_top(4)
                .margin_bottom(4)
                .build();

            let row = gtk::ListBoxRow::builder()
                .child(&label)
                .build();
            // Store the email in the widget name for retrieval on activation
            row.set_widget_name(&format!("contact:{}", contact.email));
            listbox_for_changed.append(&row);
        }

        popover_for_changed.popup();
    });
}

glib::wrapper! {
    pub struct MqComposeWindow(ObjectSubclass<imp::MqComposeWindow>)
        @extends adw::Window, gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
            gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl MqComposeWindow {
    pub fn new(parent: &impl IsA<gtk::Window>) -> Self {
        let win: Self = glib::Object::builder().build();
        win.set_transient_for(Some(parent));
        win
    }

    /// Returns true if any compose field has content (used for close confirmation).
    pub fn has_content(&self) -> bool {
        let has_to = !self.to_addresses().is_empty();
        let has_subject = !self.subject().is_empty();
        let has_body = {
            // Check cached HTML first (rich editor)
            let html = self.imp().cached_body_html.borrow().clone();
            if !html.is_empty() {
                let plain = html_to_plain_text(&html);
                let trimmed = plain.trim();
                !trimmed.is_empty() && !trimmed.starts_with("-- \n") && trimmed != "--"
            } else {
                let text = self.body_text();
                let trimmed = text.trim();
                !trimmed.is_empty() && !trimmed.starts_with("-- \n")
            }
        };
        let has_attachments = !self.attachments().is_empty();
        has_to || has_subject || has_body || has_attachments
    }

    /// Mark this compose window as having sent successfully (skips close dialog).
    pub fn set_sent_successfully(&self) {
        self.imp().sent_successfully.set(true);
    }

    /// Get the draft ID if this compose is backed by a saved draft.
    pub fn draft_id(&self) -> Option<i64> {
        *self.imp().draft_id.borrow()
    }

    /// Set the draft ID.
    pub fn set_draft_id(&self, id: i64) {
        *self.imp().draft_id.borrow_mut() = Some(id);
    }

    /// Set the callback invoked when the user chooses "Save Draft" on close.
    pub fn set_save_draft_callback<F: Fn(&Self) + 'static>(&self, f: F) {
        *self.imp().save_draft_callback.borrow_mut() = Some(Box::new(f));
    }

    /// Set the available contacts for autocomplete in address fields.
    pub fn set_contacts(&self, contacts: Vec<ContactEntry>) {
        *self.imp().contacts.borrow_mut() = contacts;
    }

    /// Set the available accounts in the From dropdown.
    /// Returns the list of account IDs in dropdown order.
    pub fn set_accounts(&self, accounts: &[(i64, String)]) {
        let imp = self.imp();
        let labels: Vec<&str> = accounts.iter().map(|(_, email)| email.as_str()).collect();
        if let Some(dropdown) = imp.from_dropdown.borrow().as_ref() {
            dropdown.set_model(Some(&gtk::StringList::new(&labels)));
        }
        *imp.accounts.borrow_mut() = accounts.to_vec();
    }

    /// Select a specific account in the From dropdown by account ID.
    pub fn select_account(&self, account_id: i64) {
        let imp = self.imp();
        let accounts = imp.accounts.borrow();
        if let Some(pos) = accounts.iter().position(|(id, _)| *id == account_id) {
            if let Some(dropdown) = imp.from_dropdown.borrow().as_ref() {
                dropdown.set_selected(pos as u32);
            }
        }
    }

    /// Get the currently selected account ID.
    pub fn selected_account(&self) -> Option<(i64, String)> {
        let imp = self.imp();
        let accounts = imp.accounts.borrow();
        if let Some(dropdown) = imp.from_dropdown.borrow().as_ref() {
            let idx = dropdown.selected() as usize;
            return accounts.get(idx).cloned();
        }
        None
    }

    /// Set the To field.
    pub fn set_to(&self, to: &str) {
        if let Some(entry) = self.imp().to_entry.borrow().as_ref() {
            entry.set_text(to);
        }
    }

    /// Set the Cc field and make it visible.
    pub fn set_cc(&self, cc: &str) {
        let imp = self.imp();
        if let Some(entry) = imp.cc_entry.borrow().as_ref() {
            entry.set_text(cc);
        }
        if !cc.is_empty() {
            if let Some(row) = imp.cc_row.borrow().as_ref() {
                row.set_visible(true);
            }
            if let Some(row) = imp.bcc_row.borrow().as_ref() {
                row.set_visible(true);
            }
        }
    }

    /// Set the Subject field.
    pub fn set_subject(&self, subject: &str) {
        if let Some(entry) = self.imp().subject_entry.borrow().as_ref() {
            entry.set_text(subject);
        }
    }

    /// Set the body text and place cursor at the beginning.
    pub fn set_body(&self, text: &str) {
        // Set body in the rich text editor WebView
        if let Some(wv) = self.imp().body_webview.borrow().as_ref() {
            // Convert plain text to HTML (preserve newlines as <br>)
            let html = text
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('\n', "<br>");
            let escaped = html.replace('\\', "\\\\").replace('\'', "\\'");
            // Wait for the WebView to finish loading before setting content
            let wv_clone = wv.clone();
            let cached = std::rc::Rc::clone(&self.imp().cached_body_html);
            wv.connect_load_changed(move |_, event| {
                if event == webkit6::LoadEvent::Finished {
                    let js = format!(
                        "document.body.innerHTML = '{}'; \
                         var range = document.createRange(); \
                         range.setStart(document.body, 0); \
                         range.collapse(true); \
                         var sel = window.getSelection(); \
                         sel.removeAllRanges(); \
                         sel.addRange(range); \
                         document.body.focus(); \
                         window.webkit.messageHandlers.mqUpdate.postMessage('html:' + document.body.innerHTML);",
                        escaped
                    );
                    wv_clone.evaluate_javascript(&js, None, None, None::<&gtk::gio::Cancellable>, |_| {});
                    *cached.borrow_mut() = html.clone();
                }
            });
        }
        // Also set in the fallback TextView
        if let Some(tv) = self.imp().body_view.borrow().as_ref() {
            let buf = tv.buffer();
            buf.set_text(text);
            let start = buf.start_iter();
            buf.place_cursor(&start);
        }
    }

    /// Get the To addresses (comma-separated string -> Vec).
    pub fn to_addresses(&self) -> Vec<String> {
        self.imp()
            .to_entry
            .borrow()
            .as_ref()
            .map(|e| parse_addresses(&e.text()))
            .unwrap_or_default()
    }

    /// Get the Cc addresses.
    pub fn cc_addresses(&self) -> Vec<String> {
        self.imp()
            .cc_entry
            .borrow()
            .as_ref()
            .map(|e| parse_addresses(&e.text()))
            .unwrap_or_default()
    }

    /// Get the Bcc addresses.
    pub fn bcc_addresses(&self) -> Vec<String> {
        self.imp()
            .bcc_entry
            .borrow()
            .as_ref()
            .map(|e| parse_addresses(&e.text()))
            .unwrap_or_default()
    }

    /// Get the subject text.
    pub fn subject(&self) -> String {
        self.imp()
            .subject_entry
            .borrow()
            .as_ref()
            .map(|e| e.text().to_string())
            .unwrap_or_default()
    }

    /// Get the body text.
    pub fn body_text(&self) -> String {
        // If we have cached HTML from the rich editor, derive plain text from it
        let cached = self.imp().cached_body_html.borrow().clone();
        if !cached.is_empty() {
            return html_to_plain_text(&cached);
        }
        self.imp()
            .body_view
            .borrow()
            .as_ref()
            .map(|tv| {
                let buf = tv.buffer();
                buf.text(&buf.start_iter(), &buf.end_iter(), false)
                    .to_string()
            })
            .unwrap_or_default()
    }

    /// Get the body as HTML (from the rich text editor).
    pub fn body_html(&self) -> String {
        self.imp().cached_body_html.borrow().clone()
    }

    /// Validate all address fields. Returns a list of error messages.
    pub fn validate_addresses(&self) -> Vec<String> {
        let mut errors = Vec::new();
        for addr in self.to_addresses() {
            if !is_valid_email(&addr) {
                errors.push(format!("Invalid To address: {addr}"));
            }
        }
        for addr in self.cc_addresses() {
            if !is_valid_email(&addr) {
                errors.push(format!("Invalid Cc address: {addr}"));
            }
        }
        for addr in self.bcc_addresses() {
            if !is_valid_email(&addr) {
                errors.push(format!("Invalid Bcc address: {addr}"));
            }
        }
        errors
    }

    /// Get the list of attached files: (filename, path).
    pub fn attachments(&self) -> Vec<(String, std::path::PathBuf)> {
        self.imp().attachments.borrow().clone()
    }

    /// Enter sending state: disable all inputs, show pulsing progress bar.
    pub fn set_sending(&self, sending: bool) {
        let imp = self.imp();
        if let Some(bar) = imp.sending_bar.borrow().as_ref() {
            bar.set_visible(sending);
            if sending {
                bar.set_text(Some("Sending\u{2026}"));
                bar.set_show_text(true);
                bar.pulse();
                // Pulse the bar periodically while sending
                let bar = bar.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
                    if bar.is_visible() {
                        bar.pulse();
                        glib::ControlFlow::Continue
                    } else {
                        glib::ControlFlow::Break
                    }
                });
            }
        }
        if let Some(btn) = imp.send_button.borrow().as_ref() {
            btn.set_sensitive(!sending);
        }
        if let Some(form) = imp.form_box.borrow().as_ref() {
            form.set_sensitive(!sending);
        }
        if let Some(scrolled) = imp.body_scrolled.borrow().as_ref() {
            scrolled.set_sensitive(!sending);
        }
        if let Some(wv) = imp.body_webview.borrow().as_ref() {
            wv.set_sensitive(!sending);
        }
    }

    /// Connect a callback for the Send button.
    pub fn connect_send<F: Fn() + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().send_button.borrow().as_ref() {
            btn.connect_clicked(move |_| f());
        }
    }

    /// Apply a compose mode: pre-fill fields for reply, reply-all, or forward.
    pub fn apply_mode(
        &self,
        mode: &ComposeMode,
        signature: &str,
        reply_position: mq_core::config::ReplyPosition,
    ) {
        let sig_block = if signature.is_empty() {
            String::new()
        } else {
            format!("\n\n-- \n{signature}")
        };

        match mode {
            ComposeMode::New => {
                self.set_title(Some("New Message"));
                if !sig_block.is_empty() {
                    self.set_body(&sig_block);
                }
            }
            ComposeMode::Reply {
                from,
                subject,
                date,
                body,
                ..
            } => {
                self.set_title(Some("Reply"));
                self.set_to(from);
                self.set_subject(&reply_subject(subject));
                let quoted = quote_body(from, date, body);
                self.set_body(&format_reply_body(&sig_block, &quoted, reply_position));
            }
            ComposeMode::ReplyAll {
                from,
                to,
                cc,
                subject,
                date,
                body,
                ..
            } => {
                self.set_title(Some("Reply All"));
                self.set_to(from);
                // Get the user's own email to exclude from CC
                let self_email = self.selected_account()
                    .map(|(_, e)| e.to_lowercase())
                    .unwrap_or_default();
                // Merge original To + Cc into Cc (excluding the sender and self).
                let extract_email = |addr: &str| -> String {
                    let addr = addr.trim();
                    if let Some(start) = addr.find('<') {
                        if let Some(end) = addr.find('>') {
                            return addr[start + 1..end].to_lowercase();
                        }
                    }
                    addr.to_lowercase()
                };
                let self_extracted = extract_email(&self_email);
                let from_extracted = extract_email(from);
                let mut all_cc: Vec<String> = to
                    .iter()
                    .chain(cc.iter())
                    .filter(|a| extract_email(a) != from_extracted)
                    .filter(|a| {
                        self_extracted.is_empty() || extract_email(a) != self_extracted
                    })
                    .cloned()
                    .collect();
                all_cc.dedup();
                if !all_cc.is_empty() {
                    self.set_cc(&all_cc.join(", "));
                }
                self.set_subject(&reply_subject(subject));
                let quoted = quote_body(from, date, body);
                self.set_body(&format_reply_body(&sig_block, &quoted, reply_position));
            }
            ComposeMode::Forward {
                from,
                subject,
                date,
                to,
                body,
            } => {
                self.set_title(Some("Forward"));
                self.set_subject(&forward_subject(subject));
                let fwd = format!(
                    "---------- Forwarded message ----------\n\
                     From: {from}\n\
                     Date: {date}\n\
                     Subject: {subject}\n\
                     To: {to}\n\
                     \n\
                     {body}"
                );
                self.set_body(&format_reply_body(&sig_block, &fwd, reply_position));
            }
        }
    }

    /// Get In-Reply-To and References headers from the compose mode.
    pub fn reply_headers(mode: &ComposeMode) -> (Option<String>, Option<String>) {
        match mode {
            ComposeMode::Reply {
                message_id,
                references,
                ..
            }
            | ComposeMode::ReplyAll {
                message_id,
                references,
                ..
            } => {
                let in_reply_to = message_id.clone();
                let new_refs = match (references, message_id) {
                    (Some(refs), Some(mid)) => Some(format!("{refs} {mid}")),
                    (None, Some(mid)) => Some(mid.clone()),
                    (Some(refs), None) => Some(refs.clone()),
                    (None, None) => None,
                };
                (in_reply_to, new_refs)
            }
            _ => (None, None),
        }
    }
}

fn parse_addresses(text: &str) -> Vec<String> {
    let mut results = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut in_angle = false;

    for ch in text.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            '<' if !in_quotes => {
                in_angle = true;
                current.push(ch);
            }
            '>' if !in_quotes => {
                in_angle = false;
                current.push(ch);
            }
            ',' if !in_quotes && !in_angle => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    results.push(trimmed);
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        results.push(trimmed);
    }
    results
}

fn reply_subject(subject: &str) -> String {
    if subject.to_lowercase().starts_with("re:") {
        subject.to_string()
    } else {
        format!("Re: {subject}")
    }
}

fn forward_subject(subject: &str) -> String {
    if subject.to_lowercase().starts_with("fwd:") {
        subject.to_string()
    } else {
        format!("Fwd: {subject}")
    }
}

/// Basic email validation: must have text@text.text form.
fn is_valid_email(addr: &str) -> bool {
    let addr = addr.trim();
    // Handle "Name <email>" format
    let email = if let Some(start) = addr.find('<') {
        if let Some(end) = addr.find('>') {
            &addr[start + 1..end]
        } else {
            addr
        }
    } else {
        addr
    };
    let parts: Vec<&str> = email.splitn(2, '@').collect();
    if parts.len() != 2 {
        return false;
    }
    let local = parts[0];
    let domain = parts[1];
    !local.is_empty() && !domain.is_empty() && domain.contains('.')
}

/// Format the compose body with signature and quoted text respecting reply position.
fn format_reply_body(
    sig_block: &str,
    quoted: &str,
    position: mq_core::config::ReplyPosition,
) -> String {
    match position {
        mq_core::config::ReplyPosition::Above => {
            // Cursor position above quoted text, signature between cursor and quote
            if sig_block.is_empty() {
                format!("\n\n{quoted}")
            } else {
                format!("{sig_block}\n\n{quoted}")
            }
        }
        mq_core::config::ReplyPosition::Below => {
            // Quoted text first, then signature at the end
            if sig_block.is_empty() {
                format!("\n\n{quoted}")
            } else {
                format!("\n\n{quoted}{sig_block}")
            }
        }
    }
}

fn quote_body(from: &str, date: &str, body: &str) -> String {
    let quoted_lines: String = body
        .lines()
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("On {date}, {from} wrote:\n{quoted_lines}")
}

/// Simple HTML to plain text conversion for compose body.
fn html_to_plain_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut last_was_block = false;

    let mut chars = html.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '<' => {
                in_tag = true;
                // Check for block-level tags that need newlines
                let tag_start: String = chars.clone().take(10).collect::<String>().to_lowercase();
                if tag_start.starts_with("br")
                    || tag_start.starts_with("/p")
                    || tag_start.starts_with("/div")
                    || tag_start.starts_with("/h")
                    || tag_start.starts_with("/li")
                    || tag_start.starts_with("/tr")
                {
                    if !last_was_block {
                        text.push('\n');
                        last_was_block = true;
                    }
                } else if tag_start.starts_with("li") {
                    if !last_was_block {
                        text.push('\n');
                    }
                    text.push_str("- ");
                    last_was_block = false;
                }
            }
            '>' if in_tag => {
                in_tag = false;
            }
            '&' if !in_tag => {
                let entity: String = chars.clone().take(10).take_while(|c| *c != ';').collect();
                match entity.as_str() {
                    "amp" => { text.push('&'); for _ in 0..4 { chars.next(); } }
                    "lt" => { text.push('<'); for _ in 0..3 { chars.next(); } }
                    "gt" => { text.push('>'); for _ in 0..3 { chars.next(); } }
                    "quot" => { text.push('"'); for _ in 0..5 { chars.next(); } }
                    "nbsp" => { text.push(' '); for _ in 0..5 { chars.next(); } }
                    "#39" => { text.push('\''); for _ in 0..4 { chars.next(); } }
                    _ => text.push('&'),
                }
                last_was_block = false;
            }
            _ if !in_tag => {
                text.push(ch);
                last_was_block = false;
            }
            _ => {}
        }
    }
    text.trim().to_string()
}

