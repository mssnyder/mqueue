//! Diagnostic test for WebKitGTK CSS rendering.
//! Run with: cargo run --example webview_test -p mq-app

use gtk::prelude::*;
use webkit6::prelude::*;

fn main() {
    let app = gtk::Application::builder()
        .application_id("com.test.webview-css")
        .build();

    app.connect_activate(|app| {
        // WORKAROUND: On Wayland, gtk-xft-dpi can be 0, which causes
        // WebKitGTK's refreshInternalScaling() to compute NaN page zoom
        // (fontDPI/96 = 0/96 = 0, then 0/0 = NaN). Set a sane default.
        if let Some(settings) = gtk::Settings::default() {
            let xft_dpi = settings.gtk_xft_dpi();
            eprintln!("gtk-xft-dpi: {} (real DPI = {})", xft_dpi, xft_dpi as f64 / 1024.0);
            if xft_dpi <= 0 {
                settings.set_gtk_xft_dpi(96 * 1024);
                eprintln!("Fixed gtk-xft-dpi to {} (96 DPI)", 96 * 1024);
            }
        }

        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("WebView CSS Diagnostic")
            .default_width(800)
            .default_height(700)
            .build();

        let wv = webkit6::WebView::new();
        wv.set_vexpand(true);
        wv.set_hexpand(true);

        let settings = webkit6::prelude::WebViewExt::settings(&wv).unwrap();
        settings.set_enable_javascript(true);

        let zoom = webkit6::prelude::WebViewExt::zoom_level(&wv);
        eprintln!("WebView zoom_level: {}", zoom);

        window.set_child(Some(&wv));
        window.present();

        let html = r#"<!DOCTYPE html>
<html><head>
<meta charset="utf-8">
<style>
body { font-family: system-ui; font-size: 16px; margin: 8px; }
.test-box {
    background: red; color: white;
    padding: 40px; margin: 20px;
    border: 10px solid blue; border-radius: 16px;
}
.card-box {
    border: 2px solid #ccc; border-radius: 8px;
    padding: 16px; margin: 12px 0; background: #f5f5f5;
}
#diag { font-family: monospace; font-size: 12px; white-space: pre-wrap; background: #222; color: #0f0; padding: 8px; margin: 8px 0; }
</style>
</head><body>
<div id="box1" class="test-box">
  CSS TEST: 40px padding, 20px margin, 10px blue border, rounded corners
</div>
<div id="box2" class="card-box">
  Card test: should have border, padding, and gray background
</div>
<div id="diag">Computing...</div>
<script>
window.addEventListener('load', function() {
    var el = document.getElementById('box1');
    var cs = window.getComputedStyle(el);
    var out = 'innerWidth=' + window.innerWidth + ' innerHeight=' + window.innerHeight + '\n';
    out += 'devicePixelRatio=' + window.devicePixelRatio + '\n';
    out += 'box1 offsetW=' + el.offsetWidth + ' offsetH=' + el.offsetHeight + '\n';
    out += 'box1 padding-top=' + cs.paddingTop + ' border-top-width=' + cs.borderTopWidth + '\n';
    out += 'box1 width=' + cs.width + '\n';
    out += 'body width=' + document.body.offsetWidth + '\n';
    document.getElementById('diag').textContent = out;
});
</script>
</body></html>"#;

        let wv_clone = wv.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
            wv_clone.load_html(html, None);
        });
    });

    app.run();
}
