use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    time::Duration,
};

use gpui::{
    AnyElement, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Timer, Window,
    div, img, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::InputState,
    select::{Select, SelectEvent, SelectItem, SelectState},
    v_flex,
};
use serde_json::Value;

use crate::assets::OrbitIcon;
use crate::model::*;
use crate::process::{ProcessBridge, TaskId};
use crate::remote_images::RemoteImageBridge;
use crate::string_rule::{StringOperationDraft, StringRuleDraft};

mod components;
mod controller;
mod pages;

pub(super) const ACTIVITY_DRAWER_TRANSITION: Duration = Duration::from_millis(180);

#[derive(Debug, Clone)]
pub(super) enum ConfirmationAction {
    LogoutAccount(String),
    RemoveYggdrasilProvider(String),
    UnregisterInstance(String),
    RemoveJavaRuntime(String),
    RemovePackage(String),
    PurgePackage(String),
    CleanOrbitCache,
    InstallModpack(PathBuf),
    AcceptEula(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeFlowMode {
    Create,
    Migrate,
    UpdateLoader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeFlowStep {
    Minecraft,
    Components,
    Review,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct RuntimeFlow {
    pub mode: RuntimeFlowMode,
    pub step: RuntimeFlowStep,
}

#[derive(Debug, Clone)]
pub(super) struct MigrationReview {
    pub source_pack: PathBuf,
    pub target: PathBuf,
    pub target_id: String,
    pub target_name: String,
    pub plan: MigrationResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AccountFlow {
    Choose,
    Offline,
    YggdrasilEndpoints,
    YggdrasilLogin,
}

#[derive(Debug, Clone, Default)]
pub(super) enum SearchState {
    #[default]
    Idle,
    Running,
    Completed,
    Failed(String),
}

#[derive(Debug, Clone)]
pub(super) struct Confirmation {
    pub title: String,
    pub body: String,
    pub action: ConfirmationAction,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ToastKind {
    Success,
    Warning,
    Danger,
}

#[derive(Debug, Clone)]
pub(super) struct Toast {
    pub message: String,
    pub kind: ToastKind,
}

#[derive(Default)]
pub(super) struct NewInstanceForm {
    pub name: String,
    pub server_directory: String,
    pub kind: usize,
    pub minecraft: String,
    pub loader: usize,
    pub loader_version: String,
}

#[derive(Debug, Clone)]
pub(super) struct PackageEditor {
    pub package: InstalledPackage,
    pub environment: String,
    pub remote_provider: usize,
    pub section: PackageEditorSection,
    pub policy: PackagePolicyDraft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PackageEditorSection {
    Numeric,
    String,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PackagePolicyMode {
    Any,
    Comparison,
    Range,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PackagePolicyOperator {
    Exact,
    GreaterThan,
    AtLeast,
    LessThan,
    AtMost,
}

impl PackagePolicyOperator {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Exact => "=",
            Self::GreaterThan => ">",
            Self::AtLeast => "≥",
            Self::LessThan => "<",
            Self::AtMost => "≤",
        }
    }

    fn command(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::GreaterThan => "greater-than",
            Self::AtLeast => "at-least",
            Self::LessThan => "less-than",
            Self::AtMost => "at-most",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct PackagePolicyDraft {
    pub mode: PackagePolicyMode,
    pub operator: PackagePolicyOperator,
    pub version: Option<String>,
    pub lower: Option<String>,
    pub upper: Option<String>,
    pub include_lower: bool,
    pub include_upper: bool,
    pub replaced_custom: Option<String>,
    pub string: StringRuleDraft,
    pub string_condition: StringOperationDraft,
    pub string_edit_index: Option<usize>,
}

impl Default for PackagePolicyDraft {
    fn default() -> Self {
        Self {
            mode: PackagePolicyMode::Any,
            operator: PackagePolicyOperator::Exact,
            version: None,
            lower: None,
            upper: None,
            include_lower: true,
            include_upper: true,
            replaced_custom: None,
            string: StringRuleDraft::default(),
            string_condition: StringOperationDraft::default(),
            string_edit_index: None,
        }
    }
}

impl PackagePolicyDraft {
    fn from_policy(policy: &PackageVersionPolicy, string: &str) -> anyhow::Result<Self> {
        let mut draft = Self::default();
        match policy {
            PackageVersionPolicy::Any => {}
            PackageVersionPolicy::Comparison { operator, version } => {
                draft.mode = PackagePolicyMode::Comparison;
                draft.operator = match operator {
                    PackageVersionOperator::Exact => PackagePolicyOperator::Exact,
                    PackageVersionOperator::GreaterThan => PackagePolicyOperator::GreaterThan,
                    PackageVersionOperator::AtLeast => PackagePolicyOperator::AtLeast,
                    PackageVersionOperator::LessThan => PackagePolicyOperator::LessThan,
                    PackageVersionOperator::AtMost => PackagePolicyOperator::AtMost,
                };
                draft.version = Some(version.clone());
            }
            PackageVersionPolicy::Range {
                lower,
                upper,
                include_lower,
                include_upper,
            } => {
                draft.mode = PackagePolicyMode::Range;
                draft.lower = Some(lower.clone());
                draft.upper = Some(upper.clone());
                draft.include_lower = *include_lower;
                draft.include_upper = *include_upper;
            }
            PackageVersionPolicy::Custom { requirement } => {
                draft.replaced_custom = Some(requirement.clone());
            }
        }
        draft.string = StringRuleDraft::parse(string)?;
        Ok(draft)
    }

    fn select_mode(&mut self, mode: PackagePolicyMode, default_version: Option<&str>) {
        self.mode = mode;
        if mode == PackagePolicyMode::Comparison
            && self.version.is_none()
            && let Some(version) = default_version
        {
            self.version = Some(version.to_string());
        }
    }

    fn command_args(&self) -> Option<Vec<String>> {
        let mut arguments = match self.mode {
            PackagePolicyMode::Any => Some(vec!["any".to_string()]),
            PackagePolicyMode::Comparison => Some(vec![
                self.operator.command().to_string(),
                self.version.clone()?,
            ]),
            PackagePolicyMode::Range => Some(vec![
                "range".to_string(),
                self.lower.clone()?,
                self.upper.clone()?,
                "--lower-bound".to_string(),
                if self.include_lower {
                    "inclusive"
                } else {
                    "exclusive"
                }
                .to_string(),
                "--upper-bound".to_string(),
                if self.include_upper {
                    "inclusive"
                } else {
                    "exclusive"
                }
                .to_string(),
            ]),
        }?;
        arguments.push("--string".to_string());
        arguments.push(self.string.expression()?);
        Some(arguments)
    }
}

#[derive(Debug, Clone)]
pub(super) struct PackageAddForm {
    pub project: SearchResult,
    pub environment: usize,
    pub optional: bool,
    pub recommended_constraint: bool,
}

impl PackageEditor {
    fn new(package: InstalledPackage) -> Self {
        Self {
            environment: package
                .configured_environment
                .clone()
                .unwrap_or_else(|| "auto".to_string()),
            package,
            remote_provider: 0,
            section: PackageEditorSection::Numeric,
            policy: PackagePolicyDraft::default(),
        }
    }
}

#[derive(Clone)]
pub(super) struct InstanceOption {
    pub id: String,
    pub title: SharedString,
}

impl SelectItem for InstanceOption {
    type Value = String;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

pub(super) struct Inputs {
    pub package_filter: Entity<InputState>,
    pub search_query: Entity<InputState>,
    pub minecraft_filter: Entity<InputState>,
    pub new_name: Entity<InputState>,
    pub new_server_directory: Entity<InputState>,
    pub import_root: Entity<InputState>,
    pub offline_name: Entity<InputState>,
    pub ygg_username: Entity<InputState>,
    pub ygg_profile: Entity<InputState>,
    pub ygg_password: Entity<InputState>,
    pub ygg_provider_id: Entity<InputState>,
    pub ygg_api_root: Entity<InputState>,
    pub server_command: Entity<InputState>,
    pub orbit_binary: Entity<InputState>,
    pub launcher_binary: Entity<InputState>,
    pub remote_locator: Entity<InputState>,
    pub string_value: Entity<InputState>,
    pub runtime_name: Entity<InputState>,
    pub audit_filter: Entity<InputState>,
    pub minecraft_move_destination: Entity<InputState>,
}

impl Inputs {
    fn new(window: &mut Window, cx: &mut Context<OrbitApp>, preferences: &Preferences) -> Self {
        fn input(
            window: &mut Window,
            cx: &mut Context<OrbitApp>,
            placeholder: impl Into<SharedString>,
        ) -> Entity<InputState> {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        }

        Self {
            package_filter: input(window, cx, tr!("Filter packages").into_owned()),
            search_query: input(window, cx, tr!("Search projects").into_owned()),
            minecraft_filter: input(window, cx, tr!("Filter Minecraft versions").into_owned()),
            new_name: input(window, cx, tr!("Installation name").into_owned()),
            new_server_directory: input(window, cx, tr!("Server directory").into_owned()),
            import_root: input(window, cx, tr!("Existing game directory").into_owned()),
            offline_name: input(window, cx, tr!("Player name").into_owned()),
            ygg_username: input(window, cx, tr!("Username").into_owned()),
            ygg_profile: input(window, cx, tr!("Profile name or UUID").into_owned()),
            ygg_password: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(tr!("Password").into_owned())
                    .masked(true)
            }),
            ygg_provider_id: input(window, cx, tr!("Endpoint name").into_owned()),
            ygg_api_root: input(window, cx, "https://auth.example.com/api/yggdrasil"),
            server_command: input(window, cx, "say Hello"),
            orbit_binary: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(preferences.orbit_binary.display().to_string())
            }),
            launcher_binary: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(preferences.launcher_binary.display().to_string())
            }),
            remote_locator: input(window, cx, tr!("Project ID or JAR path").into_owned()),
            string_value: input(window, cx, tr!("Version text").into_owned()),
            runtime_name: input(window, cx, tr!("Installation name").into_owned()),
            audit_filter: input(window, cx, tr!("Filter by mod").into_owned()),
            minecraft_move_destination: input(
                window,
                cx,
                tr!("New Minecraft directory").into_owned(),
            ),
        }
    }
}

pub struct OrbitApp {
    pub(super) preferences: Preferences,
    pub(super) bridge: ProcessBridge,
    pub(super) remote_images: RemoteImageBridge,
    pub(super) tasks: BTreeMap<TaskId, TaskView>,
    pub(super) intents: HashMap<TaskId, Intent>,
    pub(super) runtime_instances: Vec<RuntimeInstance>,
    pub(super) instance_detail: Option<RuntimeInstanceDetail>,
    pub(super) packages: Vec<InstalledPackage>,
    pub(super) package_versions: Option<PackageVersions>,
    pub(super) mod_view: usize,
    pub(super) search_results: Vec<SearchResult>,
    pub(super) search_truncated: bool,
    pub(super) search_state: SearchState,
    pub(super) package_editor: Option<PackageEditor>,
    pub(super) package_add: Option<PackageAddForm>,
    pub(super) outdated: Vec<OutdatedPackage>,
    pub(super) outdated_checked: bool,
    pub(super) outdated_diagnostics: Vec<ResolutionDiagnostic>,
    pub(super) outdated_warnings: Vec<String>,
    pub(super) accounts: Vec<Account>,
    pub(super) accounts_error: Option<String>,
    pub(super) yggdrasil_providers: Vec<YggdrasilProvider>,
    pub(super) java_runtimes: Vec<JavaRuntime>,
    pub(super) java_verification_requested: bool,
    pub(super) minecraft_versions: Vec<MinecraftVersion>,
    pub(super) launcher_config: Vec<LauncherConfigEntry>,
    pub(super) orbit_config: Vec<OrbitConfigEntry>,
    pub(super) orbit_config_path: Option<std::path::PathBuf>,
    pub(super) minecraft_directory: Option<MinecraftDirectory>,
    pub(super) latest_minecraft_release: Option<String>,
    pub(super) latest_minecraft_snapshot: Option<String>,
    pub(super) minecraft_version_type: usize,
    pub(super) loader_version_catalogs: HashMap<(String, String), Vec<LoaderVersion>>,
    pub(super) java_requirements: HashMap<String, JavaRequirement>,
    pub(super) server_status: Option<ServerStatus>,
    pub(super) audit: Option<AuditSummary>,
    pub(super) audit_min_risk: usize,
    pub(super) activity_open: bool,
    pub(super) activity_closing: bool,
    pub(super) confirmation: Option<Confirmation>,
    pub(super) interaction: Option<PendingInteraction>,
    pub(super) toast: Option<Toast>,
    pub(super) new_instance: NewInstanceForm,
    pub(super) ygg_provider: String,
    pub(super) ygg_allow_insecure_http: bool,
    pub(super) microsoft_session: Option<MicrosoftDeviceSession>,
    pub(super) eula_document: Option<Value>,
    pub(super) runtime_flow: Option<RuntimeFlow>,
    pub(super) migration_source: Option<PathBuf>,
    pub(super) migration_review: Option<MigrationReview>,
    pub(super) runtime_rename_open: bool,
    pub(super) account_flow: Option<AccountFlow>,
    pub(super) ygg_endpoint_editor_open: bool,
    pub(super) inputs: Inputs,
    pub(super) instance_select: Entity<SelectState<Vec<InstanceOption>>>,
    _subscriptions: Vec<Subscription>,
    _poll_task: Task<()>,
}

impl OrbitApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let preferences = controller::load_preferences();
        orbit_i18n::install(preferences.language);
        crate::theme::apply(window, cx, preferences.theme_mode, preferences.accent_theme);
        let inputs = Inputs::new(window, cx, &preferences);
        let instance_select = cx.new(|cx| {
            SelectState::new(Vec::<InstanceOption>::new(), None, window, cx).searchable(true)
        });
        let instance_subscription = cx.subscribe_in(
            &instance_select,
            window,
            |this, _, event: &SelectEvent<Vec<InstanceOption>>, window, cx| {
                let SelectEvent::Confirm(selected) = event;
                if let Some(id) = selected {
                    this.preferences.selected_instance = Some(id.clone());
                    this.save_preferences();
                    this.load_selected(window, cx);
                    cx.notify();
                }
            },
        );
        let appearance_subscription = cx.observe_window_appearance(window, |this, window, cx| {
            if this.preferences.theme_mode == crate::theme::ThemeMode::System {
                crate::theme::apply(
                    window,
                    cx,
                    this.preferences.theme_mode,
                    this.preferences.accent_theme,
                );
                cx.notify();
            }
        });
        let poll_task = cx.spawn_in(window, async move |weak, cx| {
            loop {
                Timer::after(std::time::Duration::from_millis(80)).await;
                let Some(entity) = weak.upgrade() else {
                    break;
                };
                if entity
                    .update_in(cx, |this, window, cx| {
                        if this.process_events(window, cx) {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        let mut app = Self {
            preferences,
            bridge: ProcessBridge::default(),
            remote_images: RemoteImageBridge::new(),
            tasks: BTreeMap::new(),
            intents: HashMap::new(),
            runtime_instances: Vec::new(),
            instance_detail: None,
            packages: Vec::new(),
            package_versions: None,
            mod_view: 0,
            search_results: Vec::new(),
            search_truncated: false,
            search_state: SearchState::Idle,
            package_editor: None,
            package_add: None,
            outdated: Vec::new(),
            outdated_checked: false,
            outdated_diagnostics: Vec::new(),
            outdated_warnings: Vec::new(),
            accounts: Vec::new(),
            accounts_error: None,
            yggdrasil_providers: Vec::new(),
            java_runtimes: Vec::new(),
            java_verification_requested: false,
            minecraft_versions: Vec::new(),
            launcher_config: Vec::new(),
            orbit_config: Vec::new(),
            orbit_config_path: None,
            minecraft_directory: None,
            latest_minecraft_release: None,
            latest_minecraft_snapshot: None,
            minecraft_version_type: 0,
            loader_version_catalogs: HashMap::new(),
            java_requirements: HashMap::new(),
            server_status: None,
            audit: None,
            audit_min_risk: 0,
            activity_open: false,
            activity_closing: false,
            confirmation: None,
            interaction: None,
            toast: None,
            new_instance: NewInstanceForm::default(),
            ygg_provider: String::new(),
            ygg_allow_insecure_http: false,
            microsoft_session: None,
            eula_document: None,
            runtime_flow: None,
            migration_source: None,
            migration_review: None,
            runtime_rename_open: false,
            account_flow: None,
            ygg_endpoint_editor_open: false,
            inputs,
            instance_select,
            _subscriptions: vec![instance_subscription, appearance_subscription],
            _poll_task: poll_task,
        };
        app.refresh_registries();
        app
    }

    pub(super) fn selected_instance(&self) -> Option<&RuntimeInstance> {
        let selected = self.preferences.selected_instance.as_deref()?;
        self.runtime_instances
            .iter()
            .find(|instance| instance.id == selected)
    }

    pub(super) fn is_server(&self) -> bool {
        self.selected_instance()
            .is_some_and(|instance| instance.kind == "server")
    }

    pub(super) fn input_value(&self, input: &Entity<InputState>, cx: &Context<Self>) -> String {
        input.read(cx).value().trim().to_string()
    }

    pub(super) fn toggle_activity(&mut self, cx: &mut Context<Self>) {
        if self.activity_open {
            self.close_activity(cx);
        } else {
            self.activity_open = true;
            self.activity_closing = false;
            cx.notify();
        }
    }

    pub(super) fn close_activity(&mut self, cx: &mut Context<Self>) {
        if !self.activity_open {
            return;
        }

        self.activity_open = false;
        self.activity_closing = true;
        cx.notify();
        cx.spawn(async move |weak, cx| {
            Timer::after(ACTIVITY_DRAWER_TRANSITION).await;
            let _ = weak.update(cx, |this, cx| {
                if !this.activity_open {
                    this.activity_closing = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn page_icon(page: Page) -> OrbitIcon {
        match page {
            Page::Home => OrbitIcon::Home,
            Page::Library => OrbitIcon::Mods,
            Page::Discover => OrbitIcon::Browse,
            Page::Audit => OrbitIcon::Audit,
            Page::Runtime => OrbitIcon::Download,
            Page::Accounts => OrbitIcon::Account,
            Page::Server => OrbitIcon::Server,
            Page::Settings => OrbitIcon::Settings,
        }
    }

    fn page_id(page: Page) -> usize {
        Page::ALL
            .iter()
            .position(|candidate| *candidate == page)
            .unwrap_or(0)
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut nav = v_flex().gap_1();
        for page in Page::ALL {
            if page == Page::Accounts {
                continue;
            }
            if page == Page::Server && !self.is_server() {
                continue;
            }
            if page == Page::Runtime {
                nav = nav.child(
                    div()
                        .pt_4()
                        .pb_1()
                        .px_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr!("SYSTEM").into_owned()),
                );
            }
            let selected = self.preferences.page == page;
            nav = nav.child(
                Button::new(("nav", Self::page_id(page)))
                    .icon(Self::page_icon(page))
                    .label(page.label().into_owned())
                    .large()
                    .ghost()
                    .selected(selected)
                    .w_full()
                    .justify_start()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.preferences.page = page;
                        this.save_preferences();
                        cx.notify();
                    })),
            );
        }

        let selected_account_id = self
            .instance_detail
            .as_ref()
            .and_then(|detail| detail.selected_account_id.as_deref());
        let active_account = selected_account_id
            .and_then(|id| self.accounts.iter().find(|account| account.id == id))
            .or_else(|| self.accounts.iter().find(|account| account.is_default));
        let accounts_selected = self.preferences.page == Page::Accounts;
        let account_entry = div()
            .id("sidebar-account")
            .w_full()
            .p_2()
            .rounded_lg()
            .border_1()
            .border_color(if accounts_selected {
                cx.theme().primary.opacity(0.45)
            } else {
                cx.theme().border
            })
            .bg(if accounts_selected {
                cx.theme().primary.opacity(0.1)
            } else {
                cx.theme().group_box
            })
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().secondary))
            .child(match active_account {
                Some(account) => h_flex()
                    .gap_2()
                    .items_center()
                    .child(components::account_avatar(
                        account.avatar_path.as_deref(),
                        pages::account_initials(&account.profile_name),
                        32.,
                        cx,
                    ))
                    .child(
                        v_flex()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .font_medium()
                                    .child(account.profile_name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(
                                        if account.authentication_state
                                            == "reauthentication-required"
                                        {
                                            cx.theme().danger
                                        } else {
                                            cx.theme().muted_foreground
                                        },
                                    )
                                    .child(
                                        if account.authentication_state
                                            == "reauthentication-required"
                                        {
                                            tr!("Sign-in expired").into_owned()
                                        } else {
                                            pages::account_provider_label(&account.provider)
                                        },
                                    ),
                            ),
                    )
                    .into_any_element(),
                None => h_flex()
                    .gap_2()
                    .items_center()
                    .child(components::icon_tile(OrbitIcon::Account, cx))
                    .child(
                        v_flex()
                            .child(
                                div()
                                    .text_sm()
                                    .font_medium()
                                    .child(tr!("Add account").into_owned()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(tr!("Required for client login").into_owned()),
                            ),
                    )
                    .into_any_element(),
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.preferences.page = Page::Accounts;
                this.account_flow = None;
                this.save_preferences();
                cx.notify();
            }));

        v_flex()
            .w(px(202.))
            .h_full()
            .flex_shrink_0()
            .p_3()
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .bg(cx.theme().sidebar)
            .child(
                h_flex()
                    .h(px(54.))
                    .px_2()
                    .gap_3()
                    .items_center()
                    .child(img("images/orbit.png").size(px(38.)).flex_shrink_0())
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(div().font_semibold().text_lg().child("ORBIT"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(tr!("Minecraft workspace").into_owned()),
                            ),
                    ),
            )
            .child(nav.flex_1())
            .child(account_entry)
            .into_any_element()
    }

    fn render_topbar(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .h(px(58.))
            .flex_shrink_0()
            .px_5()
            .gap_3()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                div()
                    .text_lg()
                    .font_semibold()
                    .child(self.preferences.page.label().into_owned()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Select::new(&self.instance_select)
                            .placeholder(tr!("Select installation").into_owned())
                            .search_placeholder(tr!("Search installations").into_owned())
                            .w(px(285.)),
                    )
                    .child(
                        Button::new("refresh")
                            .icon(OrbitIcon::Refresh)
                            .ghost()
                            .tooltip(tr!("Refresh").into_owned())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.refresh_registries();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("activity")
                            .icon(OrbitIcon::Activity)
                            .label(tr!("Activity").into_owned())
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_activity(cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}

impl Render for OrbitApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = pages::render(self, window, cx);
        let shell = h_flex().size_full().child(self.render_sidebar(cx)).child(
            v_flex()
                .min_w_0()
                .flex_1()
                .h_full()
                .child(self.render_topbar(cx))
                .child(div().min_h_0().flex_1().child(page))
                .when(!self.tasks.is_empty(), |this| {
                    this.child(pages::activity::render_strip(self, cx))
                }),
        );

        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(shell)
            .children(pages::activity::render_overlays(self, window, cx))
    }
}

#[cfg(test)]
mod package_policy_tests {
    use super::*;

    #[test]
    fn comparison_policy_maps_to_a_structured_cli_command() {
        let draft = PackagePolicyDraft {
            mode: PackagePolicyMode::Comparison,
            operator: PackagePolicyOperator::AtLeast,
            version: Some("1.2.3".to_string()),
            ..PackagePolicyDraft::default()
        };

        assert_eq!(
            draft.command_args().unwrap(),
            ["at-least", "1.2.3", "--string", "all"]
        );
    }

    #[test]
    fn range_policy_keeps_each_bound_inclusion_explicit() {
        let draft = PackagePolicyDraft {
            mode: PackagePolicyMode::Range,
            lower: Some("1.2.0".to_string()),
            upper: Some("2.0.0".to_string()),
            include_lower: true,
            include_upper: false,
            ..PackagePolicyDraft::default()
        };

        assert_eq!(
            draft.command_args().unwrap(),
            [
                "range",
                "1.2.0",
                "2.0.0",
                "--lower-bound",
                "inclusive",
                "--upper-bound",
                "exclusive",
                "--string",
                "all"
            ]
        );
    }

    #[test]
    fn incomplete_policy_cannot_be_applied() {
        let draft = PackagePolicyDraft {
            mode: PackagePolicyMode::Comparison,
            ..PackagePolicyDraft::default()
        };
        assert!(draft.command_args().is_none());
    }

    #[test]
    fn machine_policy_is_decoded_without_parsing_constraint_text() {
        let policy: PackageVersionPolicy = serde_json::from_value(serde_json::json!({
            "kind": "range",
            "lower": "1.2.0",
            "upper": "2.0.0",
            "include_lower": false,
            "include_upper": true
        }))
        .unwrap();
        let draft =
            PackagePolicyDraft::from_policy(&policy, "all; intersect not contains(i\"beta\")")
                .unwrap();

        assert_eq!(draft.mode, PackagePolicyMode::Range);
        assert!(!draft.include_lower);
        assert!(draft.include_upper);
        assert_eq!(draft.lower.as_deref(), Some("1.2.0"));
        assert_eq!(draft.upper.as_deref(), Some("2.0.0"));
        assert_eq!(
            draft.string.expression().as_deref(),
            Some("all; intersect not contains(i\"beta\")")
        );
    }

    #[test]
    fn unknown_machine_policy_is_rejected_instead_of_guessed() {
        assert!(
            serde_json::from_value::<PackageVersionPolicy>(serde_json::json!({
                "kind": "legacy_text",
                "requirement": "^1.2"
            }))
            .is_err()
        );
    }
}
