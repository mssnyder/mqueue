//! Message view widget for displaying email content.
//!
//! Supports:
//! - WebKitGTK-based HTML email rendering (sandboxed, JS disabled)
//! - Single message view with quoted text hiding
//! - Gmail-style threaded conversation view with collapsible messages
//! - Privacy banners (images blocked, tracking pixels, unsubscribe)

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use webkit6::prelude::*;

/// Cached content for re-rendering on theme change.
#[derive(Debug, Clone, Default)]
enum LastContent {
    #[default]
    None,
    Single { html: String, text: String },
    Thread(Vec<(String, String, String, String, bool)>),
}

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct MqMessageView {
        pub from_label: RefCell<Option<gtk::Label>>,
        pub to_label: RefCell<Option<gtk::Label>>,
        pub date_label: RefCell<Option<gtk::Label>>,
        pub subject_label: RefCell<Option<gtk::Label>>,
        pub web_view: RefCell<Option<webkit6::WebView>>,
        pub unsub_button: RefCell<Option<gtk::Button>>,
        pub placeholder: RefCell<Option<adw::StatusPage>>,
        pub content_box: RefCell<Option<gtk::Box>>,
        pub star_button: RefCell<Option<gtk::ToggleButton>>,
        pub read_button: RefCell<Option<gtk::ToggleButton>>,
        pub archive_button: RefCell<Option<gtk::Button>>,
        pub delete_button: RefCell<Option<gtk::Button>>,
        pub reply_button: RefCell<Option<gtk::Button>>,
        pub reply_all_button: RefCell<Option<gtk::Button>>,
        pub forward_button: RefCell<Option<gtk::Button>>,
        pub loading_spinner: RefCell<Option<gtk::Spinner>>,
        // Privacy UI elements
        pub images_banner: RefCell<Option<gtk::Box>>,
        pub images_blocked_label: RefCell<Option<gtk::Label>>,
        pub load_images_button: RefCell<Option<gtk::Button>>,
        pub always_load_button: RefCell<Option<gtk::Button>>,
        pub tracking_label: RefCell<Option<gtk::Label>>,
        /// Attachments section (between privacy banners and body)
        pub attachments_box: RefCell<Option<gtk::Box>>,
        /// Container for thread message cards (between privacy banners and body)
        pub thread_container: RefCell<Option<gtk::Box>>,
        /// Container for the quoted text toggle and hidden quote
        pub quote_box: RefCell<Option<gtk::Box>>,
        /// Button to show/hide the sidebar (message list) when the split view
        /// is collapsed. Only visible in narrow layouts.
        pub sidebar_button: RefCell<Option<gtk::ToggleButton>>,
        /// Track whether user explicitly loaded images for the current message.
        /// Prevents sync refresh from reverting the unblocked state.
        pub images_force_loaded: RefCell<bool>,
        /// DB id of the currently displayed message (to detect when message changes).
        pub current_message_id: RefCell<i64>,
        /// Last loaded content for re-rendering on theme change.
        /// Single: (html, text), Thread: vec of (from, date, html, text)
        pub(super) last_content: RefCell<LastContent>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MqMessageView {
        const NAME: &'static str = "MqMessageView";
        type Type = super::MqMessageView;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for MqMessageView {
        fn constructed(&self) {
            self.parent_constructed();

            let widget = self.obj();
            widget.set_orientation(gtk::Orientation::Vertical);

            // Header bar with action buttons
            let header = adw::HeaderBar::builder()
                .show_start_title_buttons(false)
                .build();
            header.set_title_widget(Some(&adw::WindowTitle::new("Message", "")));

            // Sidebar toggle — only visible when the overlay split collapses
            let sidebar_button = gtk::ToggleButton::builder()
                .icon_name("go-previous-symbolic")
                .tooltip_text("Show message list")
                .visible(false)
                .build();
            header.pack_start(&sidebar_button);

            // Action buttons in header bar
            let star_button = gtk::ToggleButton::builder()
                .icon_name("starred-symbolic")
                .tooltip_text("Star")
                .css_classes(["flat"])
                .build();

            let read_button = gtk::ToggleButton::builder()
                .icon_name("mail-read-symbolic")
                .tooltip_text("Mark as read/unread")
                .css_classes(["flat", "read-toggle"])
                .build();

            let archive_button = gtk::Button::builder()
                .icon_name("folder-symbolic")
                .tooltip_text("Archive")
                .css_classes(["flat"])
                .build();

            let delete_button = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text("Delete")
                .css_classes(["flat"])
                .build();

            let reply_button = gtk::Button::builder()
                .icon_name("mail-reply-sender-symbolic")
                .tooltip_text("Reply")
                .build();

            let reply_all_button = gtk::Button::builder()
                .icon_name("mail-reply-all-symbolic")
                .tooltip_text("Reply All")
                .build();

            let forward_button = gtk::Button::builder()
                .icon_name("mail-forward-symbolic")
                .tooltip_text("Forward")
                .build();

            header.pack_end(&delete_button);
            header.pack_end(&archive_button);
            header.pack_end(&read_button);
            header.pack_end(&star_button);
            header.pack_start(&reply_button);
            header.pack_start(&reply_all_button);
            header.pack_start(&forward_button);

            widget.append(&header);

            // Stack: placeholder (no message selected) vs content vs loading
            let stack = gtk::Stack::builder().vexpand(true).build();

            // Placeholder
            let placeholder = adw::StatusPage::builder()
                .icon_name("mail-read-symbolic")
                .title("No message selected")
                .description("Select a message from the list to read it.")
                .vexpand(true)
                .build();
            stack.add_named(&placeholder, Some("placeholder"));

            // Loading spinner
            let loading_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .valign(gtk::Align::Center)
                .halign(gtk::Align::Center)
                .vexpand(true)
                .spacing(12)
                .build();
            let spinner = gtk::Spinner::builder()
                .spinning(true)
                .width_request(32)
                .height_request(32)
                .build();
            loading_box.append(&spinner);
            loading_box.append(&gtk::Label::new(Some("Loading message\u{2026}")));
            stack.add_named(&loading_box, Some("loading"));

            // Content area — no outer ScrolledWindow because WebKitGTK
            // handles its own scrolling internally.
            let content = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(8)
                .vexpand(true)
                .build();

            // Header + banners wrapper (non-scrolling, above the WebView)
            let header_wrap = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(8)
                .margin_top(16)
                .margin_start(16)
                .margin_end(16)
                .build();

            // Email header section
            let header_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(4)
                .css_classes(["card"])
                .margin_bottom(8)
                .build();

            let header_inner = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(4)
                .margin_top(12)
                .margin_bottom(12)
                .margin_start(12)
                .margin_end(12)
                .build();

            // Subject
            let subject_label = gtk::Label::builder()
                .xalign(0.0)
                .wrap(true)
                .css_classes(["title-2"])
                .selectable(true)
                .build();
            header_inner.append(&subject_label);

            // From
            let from_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(4)
                .build();
            from_box.append(
                &gtk::Label::builder()
                    .label("From:")
                    .css_classes(["dim-label"])
                    .build(),
            );
            let from_label = gtk::Label::builder()
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .selectable(true)
                .build();
            from_box.append(&from_label);
            header_inner.append(&from_box);

            // To
            let to_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(4)
                .build();
            to_box.append(
                &gtk::Label::builder()
                    .label("To:")
                    .css_classes(["dim-label"])
                    .build(),
            );
            let to_label = gtk::Label::builder()
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .selectable(true)
                .build();
            to_box.append(&to_label);
            header_inner.append(&to_box);

            // Date
            let date_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(4)
                .build();
            date_box.append(
                &gtk::Label::builder()
                    .label("Date:")
                    .css_classes(["dim-label"])
                    .build(),
            );
            let date_label = gtk::Label::builder()
                .xalign(0.0)
                .hexpand(true)
                .selectable(true)
                .build();
            date_box.append(&date_label);
            header_inner.append(&date_box);

            header_box.append(&header_inner);

            // Unsubscribe button (hidden by default)
            let unsub_button = gtk::Button::builder()
                .label("Unsubscribe")
                .css_classes(["destructive-action"])
                .halign(gtk::Align::Start)
                .visible(false)
                .margin_start(12)
                .margin_bottom(8)
                .build();
            header_box.append(&unsub_button);

            header_wrap.append(&header_box);

            // Privacy: Images blocked banner (hidden by default)
            let images_banner = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .css_classes(["card"])
                .margin_bottom(8)
                .visible(false)
                .build();

            let banner_inner = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .margin_top(8)
                .margin_bottom(8)
                .margin_start(12)
                .margin_end(12)
                .hexpand(true)
                .build();

            let shield_icon = gtk::Image::builder()
                .icon_name("security-high-symbolic")
                .build();
            banner_inner.append(&shield_icon);

            let images_blocked_label = gtk::Label::builder()
                .label("Remote images blocked")
                .xalign(0.0)
                .hexpand(true)
                .build();
            banner_inner.append(&images_blocked_label);

            let load_images_button = gtk::Button::builder()
                .label("Load images")
                .css_classes(["flat"])
                .build();
            banner_inner.append(&load_images_button);

            let always_load_button = gtk::Button::builder()
                .label("Always from this sender")
                .css_classes(["flat"])
                .build();
            banner_inner.append(&always_load_button);

            images_banner.append(&banner_inner);
            header_wrap.append(&images_banner);

            // Privacy: Tracking pixel count (hidden by default)
            let tracking_label = gtk::Label::builder()
                .xalign(0.0)
                .css_classes(["dim-label", "caption"])
                .visible(false)
                .margin_bottom(4)
                .margin_start(16)
                .build();
            header_wrap.append(&tracking_label);

            // Attachments section (hidden by default)
            let attachments_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(4)
                .css_classes(["card"])
                .margin_bottom(8)
                .visible(false)
                .build();
            header_wrap.append(&attachments_box);

            content.append(&header_wrap);

            // Thread container (unused with WebView but kept for API compat)
            let thread_container = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(8)
                .visible(false)
                .build();

            // WebKitGTK view for HTML email rendering — handles its own scrolling
            let web_view = create_email_webview();
            web_view.set_vexpand(true);
            web_view.set_hexpand(true);
            content.append(&web_view);

            // Quoted text box (unused with WebView but kept for API compat)
            let quote_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(4)
                .visible(false)
                .build();

            stack.add_named(&content, Some("content"));

            // Show placeholder by default
            stack.set_visible_child_name("placeholder");

            widget.append(&stack);

            // Store references
            *self.from_label.borrow_mut() = Some(from_label);
            *self.to_label.borrow_mut() = Some(to_label);
            *self.date_label.borrow_mut() = Some(date_label);
            *self.subject_label.borrow_mut() = Some(subject_label);
            *self.web_view.borrow_mut() = Some(web_view);
            *self.unsub_button.borrow_mut() = Some(unsub_button);
            *self.placeholder.borrow_mut() = Some(placeholder);
            *self.content_box.borrow_mut() = Some(content);
            *self.star_button.borrow_mut() = Some(star_button);
            *self.read_button.borrow_mut() = Some(read_button);
            *self.archive_button.borrow_mut() = Some(archive_button);
            *self.delete_button.borrow_mut() = Some(delete_button);
            *self.reply_button.borrow_mut() = Some(reply_button);
            *self.reply_all_button.borrow_mut() = Some(reply_all_button);
            *self.forward_button.borrow_mut() = Some(forward_button);
            *self.loading_spinner.borrow_mut() = Some(spinner);
            *self.images_banner.borrow_mut() = Some(images_banner);
            *self.images_blocked_label.borrow_mut() = Some(images_blocked_label);
            *self.load_images_button.borrow_mut() = Some(load_images_button);
            *self.always_load_button.borrow_mut() = Some(always_load_button);
            *self.tracking_label.borrow_mut() = Some(tracking_label);
            *self.attachments_box.borrow_mut() = Some(attachments_box);
            *self.thread_container.borrow_mut() = Some(thread_container);
            *self.quote_box.borrow_mut() = Some(quote_box);
            *self.sidebar_button.borrow_mut() = Some(sidebar_button);
        }
    }

    impl WidgetImpl for MqMessageView {}
    impl BoxImpl for MqMessageView {}
}

glib::wrapper! {
    pub struct MqMessageView(ObjectSubclass<imp::MqMessageView>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl MqMessageView {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    /// Show the email with full details.
    pub fn show_message(
        &self,
        from: &str,
        to: &str,
        date: &str,
        subject: &str,
        body_text: &str,
        has_unsubscribe: bool,
        is_flagged: bool,
        is_read: bool,
        message_id: i64,
    ) {
        let imp = self.imp();

        // Track current message and reset force-loaded state on message change
        let prev_id = *imp.current_message_id.borrow();
        *imp.current_message_id.borrow_mut() = message_id;
        if prev_id != message_id {
            *imp.images_force_loaded.borrow_mut() = false;
        }

        if let Some(label) = imp.from_label.borrow().as_ref() {
            label.set_label(from);
        }
        if let Some(label) = imp.to_label.borrow().as_ref() {
            label.set_label(to);
        }
        if let Some(label) = imp.date_label.borrow().as_ref() {
            label.set_label(&mq_core::email::format_display_date(date));
        }
        if let Some(label) = imp.subject_label.borrow().as_ref() {
            label.set_label(subject);
        }
        // Show snippet as placeholder while body loads
        self.set_body_text(body_text);

        if let Some(btn) = imp.unsub_button.borrow().as_ref() {
            btn.set_visible(has_unsubscribe);
        }

        // Update toggle button states (without triggering signal handlers)
        if let Some(btn) = imp.star_button.borrow().as_ref() {
            btn.set_active(is_flagged);
        }
        if let Some(btn) = imp.read_button.borrow().as_ref() {
            btn.set_active(is_read);
        }

        // Reset privacy banners and attachments (will be updated after body load)
        self.hide_images_banner();
        self.hide_tracking_info();
        self.hide_attachments();

        // Hide thread container (single message mode)
        if let Some(tc) = imp.thread_container.borrow().as_ref() {
            tc.set_visible(false);
        }

        // Reset quote box
        if let Some(qb) = imp.quote_box.borrow().as_ref() {
            qb.set_visible(false);
        }

        self.show_content();
    }

    /// Returns true if the user has clicked "Load images" for the current message.
    pub fn images_force_loaded(&self) -> bool {
        *self.imp().images_force_loaded.borrow()
    }

    /// Mark the current message as having images force-loaded by the user.
    pub fn set_images_force_loaded(&self) {
        *self.imp().images_force_loaded.borrow_mut() = true;
    }

    /// Show a loading spinner while fetching the message body.
    pub fn show_loading(&self) {
        if let Some(stack) = self.find_stack() {
            stack.set_visible_child_name("loading");
        }
    }

    /// Show the placeholder (no message selected).
    pub fn show_placeholder(&self) {
        if let Some(stack) = self.find_stack() {
            stack.set_visible_child_name("placeholder");
        }
    }

    /// Show the content view.
    fn show_content(&self) {
        if let Some(stack) = self.find_stack() {
            stack.set_visible_child_name("content");
        }
    }

    /// Set the body as plain text (wrapped in minimal HTML for WebKitGTK).
    pub fn set_body_text(&self, text: &str) {
        // Cache for theme change re-render
        *self.imp().last_content.borrow_mut() = LastContent::Single {
            html: String::new(),
            text: text.to_string(),
        };

        let escaped = text
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");

        // Split plain text at reply attribution ("On ... wrote:" or "> " lines)
        let (main_text, quoted_text) = split_plain_text_quote(&escaped);

        let main_html = main_text.replace('\n', "<br>");
        let body_html = if let Some(quoted) = quoted_text {
            let quoted_html = quoted.replace('\n', "<br>");
            format!(
                "<div style=\"white-space:pre-wrap;word-wrap:break-word;\">{main_html}</div>\
                 <details class=\"mq-quote\"><summary>\u{25b6} Show quoted text</summary>\
                 <div class=\"mq-quote-content\" style=\"white-space:pre-wrap;word-wrap:break-word;\">\
                 {quoted_html}</div></details>"
            )
        } else {
            format!(
                "<div style=\"white-space:pre-wrap;word-wrap:break-word;\">{main_html}</div>"
            )
        };

        let base_style = webview_base_style(false);
        let html = format!(
            "<!DOCTYPE html><html><head>{base_style}</head><body>{body_html}</body></html>"
        );
        self.load_html_into_webview(&html);

        if let Some(qb) = self.imp().quote_box.borrow().as_ref() {
            qb.set_visible(false);
        }
    }

    /// Set the body as sanitized HTML (rendered directly in WebKitGTK).
    pub fn set_body_html(&self, html: &str) {
        // Cache for theme change re-render
        *self.imp().last_content.borrow_mut() = LastContent::Single {
            html: html.to_string(),
            text: String::new(),
        };

        // Wrap quoted reply history in collapsible <details> tags
        let collapsed = collapse_quoted_html(html);
        // Inject our base styles into the HTML
        let styled = inject_base_style(&collapsed);
        self.load_html_into_webview(&styled);

        // Hide quote box — HTML emails handle quoting internally via details/summary
        if let Some(qb) = self.imp().quote_box.borrow().as_ref() {
            qb.set_visible(false);
        }
    }

    /// Set the body to show a Gmail-style collapsible conversation thread.
    ///
    /// Each entry is `(from, date, body_html, body_text)`, ordered oldest-first.
    /// The latest message (last) is expanded; earlier messages show collapsed
    /// cards with sender + snippet that can be clicked to expand.
    pub fn set_conversation(&self, messages: &[(String, String, String, String, bool)]) {
        let imp = self.imp();

        // Cache for theme change re-render
        *imp.last_content.borrow_mut() = LastContent::Thread(messages.to_vec());

        if messages.len() <= 1 {
            // Single message — just use the webview directly
            if let Some((_, _, html, text, _)) = messages.first() {
                if !html.is_empty() {
                    self.set_body_html(html);
                } else {
                    self.set_body_text(text);
                }
            }
            return;
        }

        // Detect dark mode from the GTK style manager
        let is_dark = adw::StyleManager::default().is_dark();

        // Build a single HTML document containing all thread messages
        // using <details>/<summary> for collapsing (works without JS).
        let mut full_html = String::new();
        full_html.push_str("<!DOCTYPE html><html><head>");
        full_html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
        full_html.push_str("<style>");
        // Common styles shared between light and dark
        full_html.push_str(concat!(
            "body { font-family: system-ui, -apple-system, sans-serif; font-size: 18px; ",
            "margin: 0; padding: 16px; line-height: 1.5; }",
            "details.thread-card > summary { padding: 20px 24px !important; cursor: pointer; ",
            "display: flex !important; justify-content: space-between; align-items: center; ",
            "list-style: none; font-size: 18px !important; ",
            "min-height: 60px !important; box-sizing: border-box; }",
            "details.thread-card > summary::-webkit-details-marker { display: none; }",
            ".thread-from { font-weight: 600; flex: 1; overflow: hidden; ",
            "text-overflow: ellipsis; white-space: nowrap; font-size: 18px !important; }",
            ".thread-date { font-size: 15px !important; margin-left: 16px; white-space: nowrap; }",
            ".thread-snippet { font-size: 15px; padding: 6px 24px 12px; ",
            "white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }",
            "details[open] .thread-snippet { display: none; }",
            ".thread-body { padding: 16px 24px 20px; font-size: 18px; line-height: 1.6; }",
            "img { max-width: 100%; height: auto; }",
            "details.mq-quote { margin-top: 16px; }",
            "details.mq-quote > summary { cursor: pointer; font-size: 16px; ",
            "padding: 12px 20px; border-radius: 8px; ",
            "display: inline-block; list-style: none; user-select: none; ",
            "min-height: 44px; line-height: 44px; box-sizing: border-box; font-weight: 500; }",
            "details.mq-quote > summary::-webkit-details-marker { display: none; }",
            "details.mq-quote > .mq-quote-content { margin-top: 8px; }",
            // Unread thread message styling (collapsed)
            "details.thread-card.unread > summary .thread-from { font-weight: 800; }",
            "details.thread-card.unread > summary::before { ",
            "content: ''; display: inline-block; width: 8px; height: 8px; ",
            "border-radius: 50%; background: #3584e4; margin-right: 10px; flex-shrink: 0; }",
            // When expanded, remove unread indicators (user has now seen it)
            "details.thread-card.unread[open] > summary .thread-from { font-weight: normal; }",
            "details.thread-card.unread[open] > summary::before { display: none; }",
        ));
        if is_dark {
            // Dark theme: card chrome uses explicit dark colors.
            // Email body uses per-message dark detection (class set below).
            full_html.push_str(concat!(
                "body { color: #e0e0e0; background: #1e1e1e; }",
                "details.thread-card { border: 1px solid #444; border-radius: 12px; ",
                "margin-bottom: 12px; overflow: hidden; background: #2a2a2a; }",
                "details.thread-card > summary { background: #333; color: #e0e0e0; }",
                "details.thread-card > summary:hover { background: #3a3a3a; }",
                ".thread-from { color: #fff !important; }",
                ".thread-date { color: #999; }",
                ".thread-snippet { color: #999; }",
                // Light-themed emails: override colors directly instead of CSS
                // filter inversion (which creates jarring black rectangles).
                ".thread-body.dark-invert { background: transparent; color: #e0e0e0 !important; }",
                ".thread-body.dark-invert *, .thread-body.dark-invert *[style] { ",
                "color: #e0e0e0 !important; background-color: transparent !important; }",
                ".thread-body.dark-invert a, .thread-body.dark-invert a * { color: #8ab4f8 !important; }",
                ".thread-body.dark-invert img, .thread-body.dark-invert video { }",
                // Already-dark emails: inherit dark page colors
                ".thread-body.dark-native { background: transparent; color: #e0e0e0 !important; }",
                ".thread-body.dark-native * { color: inherit !important; }",
                ".thread-body.dark-native a, .thread-body.dark-native a * { color: #8ab4f8 !important; }",
                // Center email content that has max-width set
                ".thread-body > table, .thread-body > div, .thread-body > center { ",
                "margin-left: auto !important; margin-right: auto !important; }",
                "a { color: #8ab4f8; }",
                "blockquote { border-left: 3px solid #555; margin: 8px 0; padding-left: 12px; color: #999; }",
                "details.mq-quote > summary { color: #ccc; background: rgba(255,255,255,0.08); }",
                "details.mq-quote > summary:hover { background: rgba(255,255,255,0.14); }",
            ));
        } else {
            full_html.push_str(concat!(
                "body { color: #1a1a1a; background: #fafafa; }",
                "details.thread-card { border: 1px solid #ddd; border-radius: 12px; ",
                "margin-bottom: 12px; overflow: hidden; background: #fff; }",
                "details.thread-card > summary { background: #f5f5f5; }",
                "details.thread-card > summary:hover { background: #eee; }",
                ".thread-date { color: #666; }",
                ".thread-snippet { color: #666; }",
                ".thread-body { }",
                // Center email content that has max-width set
                ".thread-body > table, .thread-body > div, .thread-body > center { ",
                "margin-left: auto !important; margin-right: auto !important; }",
                "a { color: #1a73e8; }",
                "blockquote { border-left: 3px solid #ccc; margin: 8px 0; padding-left: 12px; color: #555; }",
                "details.mq-quote > summary { color: #444; background: rgba(0,0,0,0.06); }",
                "details.mq-quote > summary:hover { background: rgba(0,0,0,0.12); }",
            ));
        }
        full_html.push_str("</style></head><body>");

        let total = messages.len();
        for (i, (from, date, html, text, is_read)) in messages.iter().enumerate() {
            let is_latest = i == total - 1;
            let escaped_from = from.replace('<', "&lt;").replace('>', "&gt;");
            let formatted_date = mq_core::email::format_display_date(date);
            let escaped_date = formatted_date.replace('<', "&lt;").replace('>', "&gt;");
            let snippet = make_card_snippet(text);
            let escaped_snippet = snippet.replace('<', "&lt;").replace('>', "&gt;");

            // Latest message is open by default
            let open = if is_latest { " open" } else { "" };
            let unread_class = if !is_read { " unread" } else { "" };
            full_html.push_str(&format!(
                "<details class=\"thread-card{unread_class}\"{open}>"
            ));
            full_html.push_str(&format!(
                "<summary>\
                 <span class=\"thread-from\">{escaped_from}</span>\
                 <span class=\"thread-date\">{escaped_date}</span></summary>"
            ));
            full_html.push_str(&format!(
                "<div class=\"thread-snippet\">{escaped_snippet}</div>"
            ));
            // Per-message dark mode: detect if the email body is already dark.
            // Plain text messages (html is empty) are rendered by us into the dark
            // theme directly, so they should never be inverted.
            let body_class = if is_dark {
                if html.is_empty() || has_dark_background(html) {
                    "thread-body dark-native"
                } else {
                    "thread-body dark-invert"
                }
            } else {
                "thread-body"
            };
            full_html.push_str(&format!("<div class=\"{body_class}\">"));

            if !html.is_empty() {
                // In thread view, strip quoted reply history — it's already
                // visible as a separate thread card, showing it again is redundant.
                let (main, _quoted) = split_html_at_quote(html);
                full_html.push_str(&strip_html_wrapper(&main));
                full_html.push_str("</div>"); // close .thread-body
                full_html.push_str("</details>"); // close .thread-card
            } else {
                let escaped = text
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                let (main_text, _quoted) = split_plain_text_quote(&escaped);
                let main_html = main_text.replace('\n', "<br>");
                full_html.push_str(&main_html);
                full_html.push_str("</div>"); // close .thread-body
                full_html.push_str("</details>"); // close .thread-card
            }
        }

        full_html.push_str("</body></html>");

        self.load_html_into_webview(&full_html);

        // Hide quote box — thread view handles everything
        if let Some(qb) = imp.quote_box.borrow().as_ref() {
            qb.set_visible(false);
        }
    }

    /// Show the "images blocked" banner with count.
    pub fn show_images_banner(&self, blocked_count: usize) {
        let imp = self.imp();
        if let Some(banner) = imp.images_banner.borrow().as_ref() {
            banner.set_visible(true);
        }
        if let Some(label) = imp.images_blocked_label.borrow().as_ref() {
            if blocked_count == 0 {
                label.set_label("Remote content blocked");
            } else if blocked_count == 1 {
                label.set_label("1 remote image blocked");
            } else {
                label.set_label(&format!("{blocked_count} remote images blocked"));
            }
        }
    }

    /// Hide the images blocked banner.
    pub fn hide_images_banner(&self) {
        if let Some(banner) = self.imp().images_banner.borrow().as_ref() {
            banner.set_visible(false);
        }
    }

    /// Show tracking pixel info.
    pub fn show_tracking_info(&self, count: usize) {
        let imp = self.imp();
        if count > 0 {
            if let Some(label) = imp.tracking_label.borrow().as_ref() {
                if count == 1 {
                    label.set_label("1 tracking pixel blocked");
                } else {
                    label.set_label(&format!("{count} tracking pixels blocked"));
                }
                label.set_visible(true);
            }
        }
    }

    /// Hide the tracking info label.
    pub fn hide_tracking_info(&self) {
        if let Some(label) = self.imp().tracking_label.borrow().as_ref() {
            label.set_visible(false);
        }
    }

    /// Display attachment rows with download buttons.
    ///
    /// Each entry: `(db_attachment_id, filename, mime_type, size_bytes)`.
    /// The `on_download` callback receives `(attachment_id, filename)`.
    pub fn set_attachments<F: Fn(i64, String) + Clone + 'static>(
        &self,
        attachments: &[(i64, String, String, Option<u64>)],
        on_download: F,
    ) {
        let imp = self.imp();
        if let Some(att_box) = imp.attachments_box.borrow().as_ref() {
            // Clear previous children
            while let Some(child) = att_box.first_child() {
                att_box.remove(&child);
            }

            if attachments.is_empty() {
                att_box.set_visible(false);
                return;
            }

            // Title row
            let title = gtk::Label::builder()
                .label(&format!("Attachments ({})", attachments.len()))
                .xalign(0.0)
                .css_classes(["heading"])
                .margin_top(8)
                .margin_start(12)
                .margin_bottom(4)
                .build();
            att_box.append(&title);

            for (att_id, filename, _mime, size) in attachments {
                let row = gtk::Box::builder()
                    .orientation(gtk::Orientation::Horizontal)
                    .spacing(8)
                    .margin_start(12)
                    .margin_end(12)
                    .margin_top(4)
                    .margin_bottom(4)
                    .build();

                let icon = gtk::Image::builder()
                    .icon_name("mail-attachment-symbolic")
                    .build();
                row.append(&icon);

                let display_name = if filename.is_empty() {
                    "Unnamed attachment".to_string()
                } else {
                    filename.clone()
                };

                let size_str = match size {
                    Some(s) if *s >= 1_048_576 => format!(" ({:.1} MB)", *s as f64 / 1_048_576.0),
                    Some(s) if *s >= 1024 => format!(" ({:.1} KB)", *s as f64 / 1024.0),
                    Some(s) => format!(" ({s} B)"),
                    None => String::new(),
                };

                let label = gtk::Label::builder()
                    .label(&format!("{display_name}{size_str}"))
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk::pango::EllipsizeMode::Middle)
                    .build();
                row.append(&label);

                let download_btn = gtk::Button::builder()
                    .icon_name("folder-download-symbolic")
                    .tooltip_text("Save attachment")
                    .css_classes(["flat"])
                    .build();

                let cb = on_download.clone();
                let att_id = *att_id;
                let fname = display_name.clone();
                download_btn.connect_clicked(move |_| {
                    cb(att_id, fname.clone());
                });
                row.append(&download_btn);

                att_box.append(&row);
            }

            att_box.set_visible(true);
        }
    }

    /// Hide the attachments section.
    pub fn hide_attachments(&self) {
        if let Some(att_box) = self.imp().attachments_box.borrow().as_ref() {
            att_box.set_visible(false);
        }
    }

    /// Connect a callback for when the star button is toggled.
    pub fn connect_star_toggled<F: Fn(bool) + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().star_button.borrow().as_ref() {
            btn.connect_toggled(move |btn| {
                f(btn.is_active());
            });
        }
    }

    /// Connect a callback for when the read button is toggled.
    pub fn connect_read_toggled<F: Fn(bool) + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().read_button.borrow().as_ref() {
            btn.connect_toggled(move |btn| {
                f(btn.is_active());
            });
        }
    }

    /// Connect a callback for the archive button.
    pub fn connect_archive_clicked<F: Fn() + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().archive_button.borrow().as_ref() {
            btn.connect_clicked(move |_| f());
        }
    }

    /// Connect a callback for the delete button.
    pub fn connect_delete_clicked<F: Fn() + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().delete_button.borrow().as_ref() {
            btn.connect_clicked(move |_| f());
        }
    }

    /// Connect a callback for the reply button.
    pub fn connect_reply_clicked<F: Fn() + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().reply_button.borrow().as_ref() {
            btn.connect_clicked(move |_| f());
        }
    }

    /// Connect a callback for the reply-all button.
    pub fn connect_reply_all_clicked<F: Fn() + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().reply_all_button.borrow().as_ref() {
            btn.connect_clicked(move |_| f());
        }
    }

    /// Connect a callback for the forward button.
    pub fn connect_forward_clicked<F: Fn() + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().forward_button.borrow().as_ref() {
            btn.connect_clicked(move |_| f());
        }
    }

    /// Connect a callback for the unsubscribe button.
    pub fn connect_unsubscribe_clicked<F: Fn() + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().unsub_button.borrow().as_ref() {
            btn.connect_clicked(move |_| f());
        }
    }

    /// Connect a callback for "Load images" button.
    pub fn connect_load_images<F: Fn() + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().load_images_button.borrow().as_ref() {
            btn.connect_clicked(move |_| f());
        }
    }

    /// Connect a callback for "Always load from this sender" button.
    pub fn connect_always_load_images<F: Fn() + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().always_load_button.borrow().as_ref() {
            btn.connect_clicked(move |_| f());
        }
    }

    /// Get the sidebar toggle button (for binding to an OverlaySplitView).
    pub fn sidebar_button(&self) -> Option<gtk::ToggleButton> {
        self.imp().sidebar_button.borrow().clone()
    }

    fn find_stack(&self) -> Option<gtk::Stack> {
        let mut child = self.first_child();
        while let Some(c) = child {
            if let Ok(stack) = c.clone().downcast::<gtk::Stack>() {
                return Some(stack);
            }
            child = c.next_sibling();
        }
        None
    }

    /// Re-render the current email content with updated theme colors.
    /// Called when the user switches between light and dark mode.
    pub fn reload_for_theme_change(&self) {
        let content = self.imp().last_content.borrow().clone();
        match content {
            LastContent::None => {}
            LastContent::Single { ref html, ref text } => {
                if !html.is_empty() {
                    self.set_body_html(html);
                } else if !text.is_empty() {
                    self.set_body_text(text);
                }
            }
            LastContent::Thread(ref msgs) => {
                self.set_conversation(msgs);
            }
        }
    }

    fn load_html_into_webview(&self, html: &str) {
        if let Some(wv) = self.imp().web_view.borrow().as_ref() {
            wv.load_html(html, None);
        }
    }
}

impl Default for MqMessageView {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// WebKitGTK setup
// ---------------------------------------------------------------------------

/// Detect whether an HTML email already has a dark background.
///
/// Checks `bgcolor` attributes and `background-color`/`background` inline
/// styles on body, table, and early container elements for dark hex colors.
fn has_dark_background(html: &str) -> bool {
    let lower = html.to_lowercase();
    // Check body, first table, first td — the common outer containers
    let check_region = &lower[..lower.len().min(3000)];

    // Look for bgcolor="..." attribute
    for attr in ["bgcolor=\"", "bgcolor='", "bgcolor = \"", "bgcolor = '"] {
        if let Some(pos) = check_region.find(attr) {
            let start = pos + attr.len();
            if let Some(color_str) = extract_color_value(&check_region[start..]) {
                if is_dark_color(&color_str) {
                    return true;
                }
            }
        }
    }

    // Look for background-color: or background: in style attributes
    for pattern in ["background-color:", "background:"] {
        let mut search_from = 0;
        while let Some(pos) = check_region[search_from..].find(pattern) {
            let abs = search_from + pos + pattern.len();
            if let Some(color_str) = extract_color_value(&check_region[abs..]) {
                if is_dark_color(&color_str) {
                    return true;
                }
            }
            search_from = abs;
        }
    }

    false
}

/// Extract a color value (hex, rgb, or named) from the start of a CSS string.
fn extract_color_value(s: &str) -> Option<String> {
    let s = s.trim_start();
    if s.starts_with('#') {
        // Hex color
        let end = s[1..].find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(s.len() - 1) + 1;
        Some(s[..end].to_string())
    } else if s.starts_with("rgb") {
        let end = s.find(')')?;
        Some(s[..end + 1].to_string())
    } else {
        // Named color — take the first word
        let end = s.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(s.len());
        if end > 0 { Some(s[..end].to_string()) } else { None }
    }
}

/// Check if a color string represents a dark color (luminance < 0.4).
fn is_dark_color(color: &str) -> bool {
    let color = color.trim();
    // Named dark colors
    match color {
        "black" | "dark" | "darkgray" | "darkgrey" | "darkslategray" | "darkslategrey"
        | "#000" | "#000000" | "#111" | "#111111" | "#222" | "#222222" | "#333" | "#333333" => {
            return true;
        }
        "white" | "transparent" | "inherit" | "initial" | "none" => return false,
        _ => {}
    }

    // Parse hex
    if let Some(stripped) = color.strip_prefix('#') {
        let (r, g, b) = if stripped.len() == 3 {
            let r = u8::from_str_radix(&stripped[0..1], 16).unwrap_or(255);
            let g = u8::from_str_radix(&stripped[1..2], 16).unwrap_or(255);
            let b = u8::from_str_radix(&stripped[2..3], 16).unwrap_or(255);
            (r * 17, g * 17, b * 17)
        } else if stripped.len() == 6 {
            let r = u8::from_str_radix(&stripped[0..2], 16).unwrap_or(255);
            let g = u8::from_str_radix(&stripped[2..4], 16).unwrap_or(255);
            let b = u8::from_str_radix(&stripped[4..6], 16).unwrap_or(255);
            (r, g, b)
        } else {
            return false;
        };
        // Relative luminance (simplified)
        let lum = 0.299 * (r as f64) + 0.587 * (g as f64) + 0.114 * (b as f64);
        return lum < 100.0; // ~0.4 on 0-255 scale
    }

    // Parse rgb(r, g, b)
    if color.starts_with("rgb") {
        let nums: Vec<u8> = color
            .trim_start_matches("rgba(")
            .trim_start_matches("rgb(")
            .trim_end_matches(')')
            .split(',')
            .filter_map(|s| s.trim().parse::<u8>().ok())
            .collect();
        if nums.len() >= 3 {
            let lum = 0.299 * (nums[0] as f64) + 0.587 * (nums[1] as f64) + 0.114 * (nums[2] as f64);
            return lum < 100.0;
        }
    }

    false
}

/// Base CSS injected into every email HTML to match the GTK theme.
///
/// When `already_dark` is true, the email has its own dark background and
/// should not be inverted even in dark mode.
fn webview_base_style(already_dark: bool) -> String {
    let is_dark = adw::StyleManager::default().is_dark();

    // Dark mode strategy:
    // - If the email already has a dark background → leave it alone
    // - Otherwise → use CSS filter invert with contrast boost
    let dark_filter = if is_dark && !already_dark {
        concat!(
            "html { background: #1e1e1e; }",
            "body { filter: invert(1) hue-rotate(180deg); }",
            // Force all text to dark pre-inversion so it becomes light after invert.
            // This fixes mid-tone grays (e.g. #999) that would otherwise be unreadable.
            "body *, body *[style] { color: #1a1a1a !important; }",
            "body a, body a * { color: #1a73e8 !important; }",
            "body img, body video, body [style*=\"background-image\"] { ",
            "filter: invert(1) hue-rotate(180deg); }",
        )
    } else if is_dark && already_dark {
        // Email is already dark — just set the page background to match
        "html { background: #1e1e1e; }"
    } else {
        ""
    };

    // Colors: when inverting, use light-mode values (they get inverted).
    // When not inverting in dark mode, use dark-mode-appropriate values.
    let (text_color, bg_color, link_color, quote_border, quote_text,
         mq_quote_color, mq_quote_bg, mq_quote_hover) = if is_dark && already_dark {
        ("#e0e0e0", "#1e1e1e", "#8ab4f8", "#555", "#999",
         "#ccc", "rgba(255,255,255,0.08)", "rgba(255,255,255,0.14)")
    } else {
        // Light mode, or dark mode with inversion (pre-inversion light colors)
        ("#1a1a1a", "#fafafa", "#1a73e8", "#ccc", "#555",
         "#444", "rgba(0,0,0,0.06)", "rgba(0,0,0,0.12)")
    };

    format!(
        concat!(
            "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
            "<style>",
            "{dark}",
            "html {{ min-height: 100%; width: 100%; }}",
            "body {{ font-family: system-ui, -apple-system, sans-serif; ",
            "font-size: 18px !important; margin: 16px !important; padding: 0; ",
            "color: {text}; background: {bg}; line-height: 1.6; ",
            "min-height: 100%; width: calc(100% - 32px); ",
            "word-wrap: break-word; overflow-wrap: break-word; box-sizing: border-box; }}",
            // Center email content that uses max-width
            "body > table, body > div, body > center {{ ",
            "margin-left: auto !important; margin-right: auto !important; }}",
            "img {{ max-width: 100% !important; height: auto !important; }}",
            "a {{ color: {link}; }}",
            "blockquote {{ border-left: 3px solid {qb}; margin: 8px 0; padding-left: 12px; color: {qt}; }}",
            "pre {{ white-space: pre-wrap; word-wrap: break-word; font-size: 16px; }}",
            "table {{ max-width: 100% !important; border-collapse: collapse; box-sizing: border-box; ",
            "width: auto !important; min-width: 0 !important; }}",
            "table[width], td[width], th[width] {{ width: auto !important; }}",
            "div, td, th {{ max-width: 100% !important; box-sizing: border-box; overflow-wrap: break-word; }}",
            "td, th {{ word-break: normal; padding: 2px 4px; }}",
            "p, li, td, th, span, div {{ font-size: inherit; }}",
            "[style*=\"width\"] {{ max-width: 100% !important; }}",
            "details.mq-quote {{ margin-top: 16px; }}",
            "details.mq-quote > summary {{ cursor: pointer; color: {mqc}; font-size: 16px; ",
            "padding: 12px 20px; background: {mqbg}; border-radius: 8px; ",
            "display: inline-block; list-style: none; user-select: none; ",
            "min-height: 44px; line-height: 44px; box-sizing: border-box; font-weight: 500; }}",
            "details.mq-quote > summary::-webkit-details-marker {{ display: none; }}",
            "details.mq-quote > summary:hover {{ background: {mqh}; }}",
            "details.mq-quote > .mq-quote-content {{ margin-top: 8px; }}",
            "</style>",
        ),
        dark = dark_filter,
        text = text_color, bg = bg_color, link = link_color,
        qb = quote_border, qt = quote_text,
        mqc = mq_quote_color, mqbg = mq_quote_bg, mqh = mq_quote_hover,
    )
}

/// Create a sandboxed WebKitGTK view configured for email display.
fn create_email_webview() -> webkit6::WebView {
    let wv = webkit6::WebView::new();

    // Sandbox: disable JS, plugins, and developer tools for email rendering
    if let Some(settings) = webkit6::prelude::WebViewExt::settings(&wv) {
        settings.set_enable_javascript(false);
        settings.set_enable_javascript_markup(false);
        settings.set_enable_developer_extras(false);
        // Match our base style's 18px font size
        settings.set_default_font_size(18);
        settings.set_default_monospace_font_size(16);
    }

    // Transparent background so the WebView blends with the GTK theme
    let transparent = gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0);
    wv.set_background_color(&transparent);

    // Intercept navigation — open links in system browser instead of in-app
    wv.connect_decide_policy(|_wv, decision, decision_type| {
        if decision_type == webkit6::PolicyDecisionType::NavigationAction {
            if let Some(nav_decision) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>()
            {
                if let Some(nav_action) = nav_decision.navigation_action() {
                    if let Some(request) = nav_action.request() {
                        if let Some(uri) = request.uri() {
                            let uri_str = uri.as_str();
                            // Allow our own content
                            if uri_str.starts_with("about:")
                                || uri_str.starts_with("data:")
                            {
                                return false; // allow
                            }
                            // Block all other navigation and open in system browser
                            if uri_str.starts_with("http://") || uri_str.starts_with("https://") {
                                // Strip tracking params before opening
                                let cleaned =
                                    mq_core::privacy::links::strip_tracking_params(uri_str);
                                if let Some(display) =
                                    gtk::gio::AppInfo::default_for_uri_scheme("https")
                                {
                                    let _ = display.launch_uris(
                                        &[&cleaned],
                                        Option::<&gtk::gio::AppLaunchContext>::None,
                                    );
                                }
                            }
                            decision.ignore();
                            return true; // handled
                        }
                    }
                }
            }
        }
        false
    });

    wv
}

/// Inject base styles into an HTML email body.
/// Ensures a `<!DOCTYPE html>` is present so WebKitGTK renders in standards mode.
fn inject_base_style(html: &str) -> String {
    let already_dark = has_dark_background(html);
    let base_style = webview_base_style(already_dark);
    // Strip any existing viewport meta tags to avoid conflicts
    let html = strip_existing_viewport(html);
    let lower = html.to_lowercase();

    // Ensure doctype for standards mode
    let has_doctype = lower.starts_with("<!doctype");
    let doctype = if has_doctype { "" } else { "<!DOCTYPE html>" };

    if let Some(pos) = lower.find("<head>") {
        let insert_at = pos + "<head>".len();
        format!(
            "{doctype}{}{base_style}{}",
            &html[..insert_at],
            &html[insert_at..]
        )
    } else if let Some(pos) = lower.find("<html") {
        // Find end of <html...> tag
        let tag_end = lower[pos..].find('>').map(|e| pos + e + 1).unwrap_or(pos + 6);
        format!(
            "{doctype}{}<head>{base_style}</head>{}",
            &html[..tag_end],
            &html[tag_end..]
        )
    } else {
        // No HTML structure — wrap it
        format!(
            "<!DOCTYPE html><html><head>{base_style}</head><body>{html}</body></html>"
        )
    }
}

/// Remove existing viewport meta tags from HTML to avoid conflicts with our injected one.
fn strip_existing_viewport(html: &str) -> String {
    let lower = html.to_lowercase();
    // Find all <meta...viewport...> tags and collect their byte ranges to skip
    let mut ranges_to_skip: Vec<(usize, usize)> = Vec::new();
    let mut search_from = 0;
    while let Some(meta_pos) = lower[search_from..].find("<meta") {
        let abs_pos = search_from + meta_pos;
        if let Some(end_offset) = lower[abs_pos..].find('>') {
            let tag = &lower[abs_pos..abs_pos + end_offset + 1];
            if tag.contains("viewport") {
                ranges_to_skip.push((abs_pos, abs_pos + end_offset + 1));
            }
            search_from = abs_pos + end_offset + 1;
        } else {
            break;
        }
    }

    if ranges_to_skip.is_empty() {
        return html.to_string();
    }

    let mut result = String::with_capacity(html.len());
    let mut last = 0;
    for (start, end) in &ranges_to_skip {
        result.push_str(&html[last..*start]);
        last = *end;
    }
    result.push_str(&html[last..]);
    result
}

/// Strip `<html>`, `<head>`, `<body>` wrapper tags from an HTML fragment,
/// keeping only the body content for embedding in a thread view.
fn strip_html_wrapper(html: &str) -> String {
    let lower = html.to_lowercase();

    // Find body content
    let start = if let Some(pos) = lower.find("<body") {
        lower[pos..].find('>').map(|end| pos + end + 1).unwrap_or(0)
    } else if let Some(pos) = lower.find("</head>") {
        // No <body> but has </head> — start after it
        pos + "</head>".len()
    } else {
        0
    };

    let end = lower.rfind("</body>").unwrap_or(html.len());

    html[start..end].to_string()
}

/// Make a short snippet for a collapsed thread card.
fn make_card_snippet(body: &str) -> String {
    let trimmed: String = body.split_whitespace().take(20).collect::<Vec<_>>().join(" ");
    if trimmed.len() < body.len() {
        format!("{trimmed}\u{2026}")
    } else {
        trimmed
    }
}

/// Split plain text email at the reply attribution line.
///
/// Returns `(main_body, Some(quoted_text))` if a quote boundary is found,
/// or `(full_text, None)` if no quote is detected.
fn split_plain_text_quote(text: &str) -> (&str, Option<&str>) {
    let lines: Vec<&str> = text.lines().collect();

    // Look for "On ... wrote:" pattern
    for (i, line) in lines.iter().enumerate() {
        let lower = line.to_lowercase();
        if (lower.contains("wrote:") || lower.contains("schrieb:"))
            && (lower.starts_with("on ") || lower.starts_with("am "))
        {
            // Find the byte offset of this line in the original text
            let offset = text
                .find(line)
                .unwrap_or(text.len());
            if offset < text.len() {
                return (&text[..offset], Some(&text[offset..]));
            }
        }

        // Outlook-style "From: ... Sent: ... Subject: ..." header block
        if lower.starts_with("from:") || lower.starts_with("from :") {
            // Check if "Sent:" and "Subject:" appear in the next few lines
            let remaining_lines = &lines[i..lines.len().min(i + 6)];
            let remaining_lower: String = remaining_lines.join("\n").to_lowercase();
            if remaining_lower.contains("sent:") && remaining_lower.contains("subject:") {
                let offset = text.find(line).unwrap_or(text.len());
                if offset < text.len() {
                    return (&text[..offset], Some(&text[offset..]));
                }
            }
        }

        // Also detect ">" quote markers — if 3+ consecutive lines start with ">"
        if i + 2 < lines.len()
            && line.starts_with("&gt; ")
            && lines[i + 1].starts_with("&gt; ")
            && lines[i + 2].starts_with("&gt; ")
        {
            let offset = text.find(line).unwrap_or(text.len());
            if offset < text.len() {
                return (&text[..offset], Some(&text[offset..]));
            }
        }
    }

    (text, None)
}

/// Find the position where quoted reply history begins in an HTML email.
///
/// Returns `Some(position)` if a quote boundary is found, `None` otherwise.
/// Detects Gmail quote divs, "On ... wrote:" patterns, Outlook "From:/Sent:/Subject:"
/// headers, and late blockquotes.
fn find_quote_start(html: &str) -> Option<usize> {
    let lower = html.to_lowercase();

    // Strategy 1: Gmail-style quote container
    if let Some(pos) = lower.find("class=\"gmail_quote\"") {
        if let Some(tag_start) = lower[..pos].rfind('<') {
            return Some(tag_start);
        }
    }

    // Strategy 2: "On ... wrote:" patterns
    let wrote_patterns = [
        "wrote:<",
        "wrote:\n",
        "wrote:<br",
        "\u{00e9}crit\u{00a0}:",
        "schrieb:",
    ];
    for pattern in &wrote_patterns {
        if let Some(pos) = lower.find(pattern) {
            return Some(find_block_boundary_before(&lower, pos));
        }
    }

    // Strategy 3: Outlook-style "From: ... Sent: ... Subject: ..." header block
    let outlook_patterns = [
        "<b>from:</b>",
        "<b>sent:</b>",
        ">from:",
        "\nfrom:",
    ];
    for pattern in &outlook_patterns {
        if let Some(pos) = lower.find(pattern) {
            let check_start = if pattern.starts_with('>') || pattern.starts_with('\n') {
                pos + 1
            } else {
                pos
            };
            let region = &lower[check_start..lower.len().min(check_start + 500)];
            if region.contains("sent:") && region.contains("subject:") {
                return Some(find_block_boundary_before(&lower, check_start));
            }
        }
    }

    // Strategy 4: Top-level blockquote in the latter portion of body
    if let Some(bq_pos) = lower.find("<blockquote") {
        let body_start = lower.find("<body").map(|p| {
            lower[p..].find('>').map(|e| p + e + 1).unwrap_or(0)
        }).unwrap_or(0);
        let body_end = lower.rfind("</body>").unwrap_or(html.len());
        let body_len = body_end.saturating_sub(body_start);
        let bq_offset = bq_pos.saturating_sub(body_start);

        if body_len > 0 && bq_offset > body_len / 3 {
            return Some(bq_pos);
        }
    }

    None
}

/// Collapse quoted reply history in a `<details>/<summary>` block (single message view).
///
/// If the quote boundary is inside a `<table>` structure, `<details>` would break,
/// so we skip collapsing in that case.
fn collapse_quoted_html(html: &str) -> String {
    let Some(split_pos) = find_quote_start(html) else {
        return html.to_string();
    };

    let quoted = &html[split_pos..];
    if quoted.len() < 50 {
        return html.to_string();
    }

    // Check if the split point is inside a <table>. If so, <details> would break
    // the table structure, so skip collapsing entirely.
    let before_lower = html[..split_pos].to_lowercase();
    let open_tables = before_lower.matches("<table").count();
    let close_tables = before_lower.matches("</table").count();
    if open_tables > close_tables {
        // Inside a table — can't use <details>, leave as-is
        return html.to_string();
    }

    let before = &html[..split_pos];
    format!(
        "{before}<details class=\"mq-quote\"><summary>\u{25b6} Show quoted text</summary>\
         <div class=\"mq-quote-content\">{quoted}</div></details>"
    )
}

/// Split HTML at the quote boundary for thread views.
///
/// Returns `(main_body, Option<quoted_section>)`. The quoted section is
/// returned separately so it can be wrapped in a `<details>` tag OUTSIDE
/// the email's own HTML structure (avoiding broken `<details>` inside `<table>`).
fn split_html_at_quote(html: &str) -> (String, Option<String>) {
    let Some(split_pos) = find_quote_start(html) else {
        return (html.to_string(), None);
    };

    let quoted = &html[split_pos..];
    if quoted.len() < 50 {
        return (html.to_string(), None);
    }

    let main = html[..split_pos].to_string();
    let quoted = html[split_pos..].to_string();
    (main, Some(quoted))
}

/// Find the nearest block-level element boundary before `pos`.
fn find_block_boundary_before(lower: &str, pos: usize) -> usize {
    let search_region = &lower[..pos];

    // Prefer <hr> tags (common before Outlook quote headers)
    if let Some(hr_pos) = search_region.rfind("<hr") {
        if pos - hr_pos < 200 {
            return hr_pos;
        }
    }

    let div_pos = search_region.rfind("<div");
    let p_pos = search_region.rfind("<p");
    let br_pos = search_region.rfind("<br");

    let candidates: Vec<usize> = [div_pos, p_pos, br_pos]
        .iter()
        .filter_map(|p| *p)
        .collect();

    if let Some(&nearest) = candidates.iter().max() {
        nearest
    } else {
        search_region.rfind('>').map(|p| p + 1).unwrap_or(0)
    }
}
