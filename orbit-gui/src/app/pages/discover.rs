use super::super::*;

impl OrbitApp {
    pub(crate) fn show_discover(&mut self, ui: &mut egui::Ui) {
        theme::section_title(
            ui,
            "Discover mods",
            "Search every configured provider through Orbit",
        );
        if self.selected_instance().is_none() {
            ui.add_space(12.0);
            if installation_required_card(
                ui,
                "Choose an installation to browse for",
                "Search results are filtered by the exact Minecraft and loader target.",
            ) {
                self.preferences.page = Page::Runtime;
            }
            return;
        }
        ui.horizontal(|ui| {
            let response = theme::text_field(
                ui,
                &mut self.search_query,
                "Search by name or project",
                theme::InputWidth::Form,
            );
            if response.changed() {
                self.search_results.clear();
                self.search_truncated = false;
                self.search_state = SearchState::Idle;
            }
            let running = matches!(self.search_state, SearchState::Running);
            if (ui
                .add_enabled(!running, theme::primary_button("Search"))
                .clicked()
                || response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                && !self.search_query.trim().is_empty()
                && !running
            {
                self.search_catalog();
            }
        });
        if self.search_truncated {
            ui.label(
                RichText::new(tr!("Results were truncated by the current limit."))
                    .color(theme::warning()),
            );
        }
        ui.add_space(10.0);
        ScrollArea::vertical().show(ui, |ui| {
            for result in self.search_results.clone() {
                theme::card().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if let Some(icon) = &result.icon_url {
                            ui.add(
                                egui::Image::new(icon)
                                    .fit_to_exact_size(Vec2::splat(58.0))
                                    .corner_radius(10),
                            );
                        } else {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(58.0), Sense::hover());
                            ui.painter().rect_filled(rect, 10, theme::accent_soft());
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                result.name.chars().next().unwrap_or('?'),
                                egui::FontId::proportional(24.0),
                                Color32::WHITE,
                            );
                        }
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&result.name).size(17.0).strong());
                                if result.name != result.slug {
                                    ui.label(
                                        RichText::new(&result.slug)
                                            .size(11.0)
                                            .color(theme::muted()),
                                    );
                                }
                                ui.label(
                                    RichText::new(&result.platform)
                                        .size(11.0)
                                        .color(theme::muted()),
                                );
                            });
                            ui.label(RichText::new(&result.description).color(theme::muted()));
                            ui.label(
                                RichText::new(tr!(
                                    "%{downloads} downloads · %{version} · %{client} / %{server}",
                                    downloads = compact_number(result.downloads),
                                    version = result.latest_version,
                                    client = result.client_side,
                                    server = result.server_side
                                ))
                                .size(12.0)
                                .color(theme::muted()),
                            );
                            ui.horizontal_wrapped(|ui| {
                                if let Some(compatible) = result.compatible {
                                    ui.label(
                                        RichText::new(if compatible {
                                            tr!("Compatible")
                                        } else {
                                            tr!("Other MC version")
                                        })
                                        .size(11.0)
                                        .color(
                                            if compatible {
                                                theme::success()
                                            } else {
                                                theme::warning()
                                            },
                                        ),
                                    );
                                }
                                for category in result.categories.iter().take(3) {
                                    ui.label(
                                        RichText::new(category).size(11.0).color(theme::muted()),
                                    );
                                }
                                if let Some(mc) = result.mc_versions.first() {
                                    ui.label(
                                        RichText::new(format!("MC {mc}"))
                                            .size(11.0)
                                            .color(theme::muted()),
                                    );
                                }
                                if let Some(accent) = result.accent_color {
                                    let color = Color32::from_rgb(
                                        ((accent >> 16) & 0xff) as u8,
                                        ((accent >> 8) & 0xff) as u8,
                                        (accent & 0xff) as u8,
                                    );
                                    let (rect, _) =
                                        ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                                    ui.painter().circle_filled(rect.center(), 4.0, color);
                                }
                            });
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.add(theme::primary_button("Add")).clicked() {
                                self.add_search_result(&result);
                            }
                        });
                    });
                });
                ui.add_space(8.0);
            }
            if self.search_results.is_empty() {
                match &self.search_state {
                    SearchState::Idle => empty_state(
                        ui,
                        "Search the catalog",
                        "Results are filtered by the selected Minecraft installation.",
                    ),
                    SearchState::Running => empty_state(
                        ui,
                        "Searching the catalog…",
                        "Orbit is querying every catalog configured for this installation.",
                    ),
                    SearchState::Completed => empty_state(
                        ui,
                        "No matching mods",
                        "Try another query or check the selected Minecraft and loader filters.",
                    ),
                    SearchState::Failed(message) => empty_state(ui, "Search failed", message),
                }
            }
        });
    }
}
