//! Pemicu tab of Webhook & API (Pego `v5/Webhook & API`): the trigger list
//! and the editor. Rules live in the settings draft and are saved with
//! "Simpan perubahan"; the test at the bottom never sends anything.

use egui::{Align, Frame, Margin, Sense, Ui, vec2};

use crate::backend::SessionView;
use crate::backend::rules::{self, Incoming, Kind, Op, Rule};
use crate::i18n::tr;
use crate::ui::fonts::{self, Weight};
use crate::ui::theme::{self, Palette};
use crate::ui::widgets::{self, rich, rich_mono};
use crate::ui::{State, icons, time};

fn host_and_path(url: &str) -> (String, String) {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    match rest.split_once('/') {
        Some((host, path)) => (host.to_owned(), format!("/{path}")),
        None => (rest.to_owned(), "/".to_owned()),
    }
}

fn numbers_label(rule: &Rule, sessions: &[SessionView]) -> String {
    if rule.sessions.is_empty() {
        return tr("All numbers", "Semua nomor").to_owned();
    }
    let names: Vec<String> = rule
        .sessions
        .iter()
        .map(|id| sessions.iter().find(|s| &s.id == id).map(super::devices::chip_label).unwrap_or_else(|| id.clone()))
        .collect();
    names.join(", ")
}

fn step(ui: &mut Ui, p: &Palette, n: u8, title: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 11.0, p.accent_soft);
        widgets::paint_text(ui, rect, &n.to_string(), fonts::ui(12.0, Weight::SemiBold), p.accent_text, Align::Center);
        ui.label(rich(title, 14.0, Weight::SemiBold, p.text));
    });
}

pub fn tab(ui: &mut Ui, p: &Palette, state: &mut State, sessions: &[SessionView]) {
    let today = time::day(time::now()) ;
    let hits = state.rule_hits.clone();
    let Some(draft) = state.settings_form.draft.as_mut() else { return };

    let mut open = None;
    let mut add = false;
    super::card(
        ui,
        tr("Webhook triggers", "Pemicu webhook"),
        tr(
            "Incoming messages that match a rule are sent by POST to the target REST API. Rules are checked from top to bottom; one message can trigger more than one rule.",
            "Pesan masuk yang cocok dengan aturan dikirim lewat POST ke REST API tujuan. Aturan dicek dari atas ke bawah; satu pesan bisa memicu lebih dari satu aturan.",
        ),
        |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let widths = [Some(52.0), None, Some(150.0), Some(220.0), Some(70.0)];
                let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
                widgets::hline(ui, head, head.bottom() - 0.5, p.border);
                let cols = widgets::columns(head, &widths, 12.0);
                for (i, title) in [tr("Active", "Aktif"), tr("Rule & condition", "Aturan & kondisi"), tr("Number", "Nomor"), tr("Target", "Tujuan"), tr("Today", "Hari ini")].iter().enumerate() {
                    let align = if i == 4 { Align::Max } else { Align::Min };
                    widgets::paint_text(ui, cols[i], title, fonts::ui(12.0, Weight::Medium), p.muted, align);
                }
                if draft.rules.is_empty() {
                    ui.add_space(14.0);
                    ui.label(rich(tr("No rules yet. Example: text starts with \"REGISTER\" → POST to your registration system.", "Belum ada aturan. Contoh: teks diawali \"REGISTER\" → POST ke sistem pendaftaran Anda."), 13.0, Weight::Regular, p.muted));
                    ui.add_space(6.0);
                }
                for (i, rule) in draft.rules.iter_mut().enumerate() {
                    let (row, response) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click());
                    let selected = state.rule_selected == Some(i);
                    if selected {
                        ui.painter().rect_filled(row, theme::r_inner(), p.selected);
                    } else if response.hovered() {
                        ui.painter().rect_filled(row, theme::r_inner(), p.raised);
                    }
                    widgets::hline(ui, row, row.bottom() - 0.5, p.border);
                    let cols = widgets::columns(row, &widths, 12.0);
                    // The switch is its own widget so it does not open the editor.
                    let mut switch = widgets::child(ui, cols[0], egui::Layout::left_to_right(Align::Center));
                    switch.add_space(6.0);
                    widgets::toggle(&mut switch, &mut rule.enabled, "");
                    let name = egui::Rect::from_min_max(cols[1].min + vec2(0.0, 10.0), egui::pos2(cols[1].right(), cols[1].top() + 28.0));
                    widgets::paint_text(ui, name, &rule.name, fonts::ui(14.0, Weight::Medium), p.text, Align::Min);
                    let cond = egui::Rect::from_min_max(egui::pos2(cols[1].left(), cols[1].top() + 30.0), egui::pos2(cols[1].right(), cols[1].top() + 46.0));
                    widgets::paint_text(ui, cond, &rule.summary(), fonts::mono(11.0), p.muted, Align::Min);
                    widgets::paint_text(ui, cols[2], &numbers_label(rule, sessions), fonts::ui(12.0, Weight::Regular), p.text_2, Align::Min);
                    let (host, path) = host_and_path(&rule.url);
                    let dest = egui::Rect::from_min_max(cols[3].min + vec2(0.0, 10.0), egui::pos2(cols[3].right(), cols[3].top() + 28.0));
                    widgets::paint_text(ui, dest, &format!("{} {path}", rule.method), fonts::mono(11.0), p.text_2, Align::Min);
                    let host_rect = egui::Rect::from_min_max(egui::pos2(cols[3].left(), cols[3].top() + 30.0), egui::pos2(cols[3].right(), cols[3].top() + 46.0));
                    widgets::paint_text(ui, host_rect, &host, fonts::mono(11.0), p.muted, Align::Min);
                    let count = hits.get(&rule.id).filter(|(day, _)| *day == today).map(|(_, n)| n.to_string()).unwrap_or_else(|| "–".to_owned());
                    widgets::paint_text(ui, cols[4], &count, fonts::mono(12.0), p.text, Align::Max);
                    if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        open = Some(i);
                    }
                }
            });
            ui.horizontal(|ui| {
                if widgets::button(ui, Some("plus"), tr("Add rule", "Tambah aturan"), widgets::Kind::Secondary).clicked() {
                    add = true;
                }
                ui.label(rich(tr("The \"Today\" count starts when the app is opened.", "Hitungan \"Hari ini\" dimulai sejak aplikasi dibuka."), 12.0, Weight::Regular, p.muted));
            });
        },
    );
    if add {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        draft.rules.push(Rule { id: format!("r{nanos}"), ..Rule::default() });
        open = Some(draft.rules.len() - 1);
    }
    if let Some(i) = open {
        state.rule_selected = if state.rule_selected == Some(i) && !add { None } else { Some(i) };
    }
    let Some(index) = state.rule_selected.filter(|i| *i < draft.rules.len()) else {
        state.rule_selected = None;
        return;
    };

    let mut remove = false;
    let mut moved: Option<isize> = None;
    let rule = &mut draft.rules[index];
    super::card(ui, &crate::trf!("Edit rule · {}", "Edit aturan · {}", rule.name), "", |ui| {
        let width = ui.available_width();
        ui.horizontal(|ui| {
            ui.label(rich(tr("Name", "Nama"), 13.0, Weight::Medium, p.text_2));
            widgets::field(ui, &mut rule.name, tr("Member registration", "Pendaftaran member"), (width - 60.0).min(420.0), false);
        });

        step(ui, p, 1, tr("If an incoming message", "Jika pesan masuk"));
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(("rule_kind", &rule.id))
                .selected_text(rule.kind.label())
                .show_ui(ui, |ui| {
                    for kind in Kind::ALL {
                        ui.selectable_value(&mut rule.kind, kind, kind.label());
                    }
                });
            egui::ComboBox::from_id_salt(("rule_op", &rule.id))
                .selected_text(rule.op.label())
                .show_ui(ui, |ui| {
                    for op in Op::ALL {
                        ui.selectable_value(&mut rule.op, op, op.label());
                    }
                });
            if rule.op != Op::Any {
                let hint = if rule.op == Op::Regex { "^(rusak|retur)" } else { "REGISTER" };
                widgets::field(ui, &mut rule.value, hint, 240.0, true);
            }
        });
        ui.horizontal(|ui| {
            widgets::toggle(ui, &mut rule.ignore_case, tr("Ignore upper/lower case", "Abaikan huruf besar/kecil"));
            ui.add_space(12.0);
            widgets::toggle(ui, &mut rule.private_only, tr("Private chats only, not groups", "Hanya chat pribadi, bukan grup"));
        });

        step(ui, p, 2, tr("Applies to numbers", "Berlaku untuk nomor"));
        let mut mode = usize::from(!rule.sessions.is_empty());
        let before = mode;
        widgets::tabs(ui, &[tr("All numbers", "Semua nomor"), tr("Specific numbers", "Nomor tertentu")], &mut mode);
        if mode != before {
            rule.sessions = if mode == 1 { sessions.first().map(|s| vec![s.id.clone()]).unwrap_or_default() } else { Vec::new() };
        }
        if mode == 1 {
            ui.horizontal_wrapped(|ui| {
                for s in sessions {
                    let mut on = rule.sessions.contains(&s.id);
                    if widgets::toggle(ui, &mut on, &super::devices::chip_label(s)).clicked() {
                        if on {
                            rule.sessions.push(s.id.clone());
                        } else {
                            rule.sessions.retain(|id| id != &s.id);
                        }
                    }
                    ui.add_space(10.0);
                }
            });
        } else {
            ui.label(rich(tr("Numbers linked later are included automatically.", "Nomor yang baru ditautkan nanti ikut otomatis."), 12.0, Weight::Regular, p.muted));
        }

        step(ui, p, 3, tr("Send to REST API", "Kirim ke REST API"));
        ui.horizontal(|ui| {
            let mut method = usize::from(rule.method == "PUT");
            widgets::tabs(ui, &["POST", "PUT"], &mut method);
            rule.method = if method == 1 { "PUT" } else { "POST" }.to_owned();
            widgets::field(ui, &mut rule.url, tr("https://crm.example.com/api/member/register", "https://crm.anda.com/api/member/register"), (ui.available_width() - 4.0).min(460.0), true);
        });
        ui.label(rich(
            tr(
                "JSON body · {{...}} is filled in automatically from the message, and signed like the webhook (X-Warust-Signature).",
                "Body JSON · {{...}} diisi otomatis dari pesan, dan ditandatangani seperti webhook (X-Warust-Signature).",
            ),
            12.0,
            Weight::Regular,
            p.muted,
        ));
        Frame::NONE
            .fill(p.raised)
            .corner_radius(theme::r_inner())
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut rule.body)
                        .frame(Frame::NONE)
                        .font(fonts::mono(12.0))
                        .desired_rows(5)
                        .desired_width(f32::INFINITY),
                );
            });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(10.0, 4.0);
            for (key, meaning) in rules::placeholders() {
                ui.label(rich_mono(key, 11.0, p.text_2)).on_hover_text(meaning);
            }
        });

        step(ui, p, 4, tr("Auto-reply to the sender", "Balas otomatis ke pengirim"));
        widgets::toggle(ui, &mut rule.reply_enabled, tr("Send a reply after the REST API answers 2xx", "Kirim balasan setelah REST API menjawab 2xx"));
        if rule.reply_enabled {
            widgets::field(ui, &mut rule.reply, tr("Thank you {{nama}}, your registration is being processed.", "Terima kasih {{nama}}, pendaftaranmu sedang kami proses."), (width - 4.0).min(620.0), false);
        }
        ui.horizontal(|ui| {
            icons::show(ui, "lock", 12.0, p.muted);
            ui.label(rich(
                tr(
                    "Always replied from the number that received the chat, through the anti-ban queue. A failed API call is retried 3 times.",
                    "Selalu dibalas dari nomor yang menerima chat, lewat antrean anti-ban. API yang gagal dicoba ulang 3 kali.",
                ),
                12.0,
                Weight::Regular,
                p.muted,
            ));
        });

        // Dry run with a sample message: nothing is sent.
        widgets::hline(ui, ui.max_rect(), ui.cursor().top(), p.border);
        ui.add_space(4.0);
        ui.label(rich(tr("Test with a sample message (nothing is sent)", "Uji dengan pesan contoh (tidak mengirim apa pun)"), 13.0, Weight::SemiBold, p.text));
        if state.rule_test.is_empty() {
            state.rule_test = Incoming::sample().text;
        }
        widgets::field(ui, &mut state.rule_test, tr("Write a sample message", "Tulis pesan contoh"), (width - 4.0).min(620.0), false);
        let sample = Incoming {
            text: state.rule_test.clone(),
            kind: match rule.kind {
                Kind::Image => "image",
                Kind::Document => "document",
                _ => "text",
            }
            .to_owned(),
            session: rule.sessions.first().cloned().unwrap_or_else(|| tr("sample", "contoh").to_owned()),
            ..Incoming::sample()
        };
        let enabled = rule.enabled;
        let check = Rule { enabled: true, ..rule.clone() };
        let (icon, verdict, color) = if check.matches(&sample) {
            ("check", if enabled {
                    tr("Match: this rule will fire", "Cocok: aturan ini akan terpicu")
                } else {
                    tr("Match, but the rule is turned off", "Cocok, tapi aturan sedang nonaktif")
                }, p.green)
        } else {
            ("x", tr("No match: the rule does not fire", "Tidak cocok: aturan tidak terpicu"), p.muted)
        };
        ui.horizontal(|ui| {
            icons::show(ui, icon, 14.0, color);
            ui.label(rich(verdict, 13.0, Weight::Medium, color));
        });
        if let Some(problem) = rule.problem() {
            ui.horizontal(|ui| {
                icons::show(ui, "triangle-alert", 14.0, p.red_text);
                ui.label(rich(&problem, 12.0, Weight::Regular, p.red_text));
            });
        }
        Frame::NONE
            .fill(p.raised)
            .corner_radius(theme::r_inner())
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(rich_mono(&format!("{} {}", rule.method, rule.url), 11.0, p.muted));
                ui.label(rich_mono(&rules::render(&rule.body, &sample, true), 12.0, p.text));
                if rule.reply_enabled {
                    ui.add_space(4.0);
                    ui.label(rich(&crate::trf!("Reply: {}", "Balasan: {}", rules::render(&rule.reply, &sample, false)), 12.0, Weight::Regular, p.text_2));
                }
            });

        ui.horizontal(|ui| {
            if widgets::button(ui, Some("x"), tr("Delete rule", "Hapus aturan"), widgets::Kind::Secondary).clicked() {
                remove = true;
            }
            if widgets::button(ui, None, tr("Move up", "Naikkan"), widgets::Kind::Secondary).clicked() {
                moved = Some(-1);
            }
            if widgets::button(ui, None, tr("Move down", "Turunkan"), widgets::Kind::Secondary).clicked() {
                moved = Some(1);
            }
            ui.label(rich(tr("Changes take effect after \"Save changes\".", "Perubahan berlaku setelah \"Simpan perubahan\"."), 12.0, Weight::Regular, p.muted));
        });
    });
    if remove {
        draft.rules.remove(index);
        state.rule_selected = None;
    } else if let Some(delta) = moved {
        let target = index as isize + delta;
        if target >= 0 && (target as usize) < draft.rules.len() {
            draft.rules.swap(index, target as usize);
            state.rule_selected = Some(target as usize);
        }
    }
}
