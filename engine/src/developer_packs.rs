use crate::capability_client::CapabilityResult;
use crate::db::capability_packs::{
    CapabilityPackApproval, CapabilityPackAuth, CapabilityPackConnector, CapabilityPackContract,
    CapabilityPackEvaluationFixture, CapabilityPackEvent, CapabilityPackHealthCheck,
    CapabilityPackManifest, CapabilityPackSkill, CapabilityPackTool, CapabilityPackUi,
    CapabilityPackUninstall,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use hmac::{Hmac, Mac};
use reqwest::{Client, Method, Url, header::HeaderMap};
use semver::Version;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

const RESPONSE_LIMIT: usize = 2 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);

pub const NATIVE_ADAPTER_IDS: &[&str] = &[
    "flamel.github.v1",
    "flamel.gitlab.v1",
    "flamel.linear.v1",
    "flamel.sentry.v1",
    "flamel.vercel.v1",
    "flamel.cloudflare.v1",
    "flamel.posthog.v1",
    "flamel.aws.v1",
    "flamel.kubernetes.v1",
    "flamel.grafana.v1",
    "flamel.gmail.v1",
    "flamel.outlook-mail.v1",
    "flamel.google-calendar.v1",
    "flamel.outlook-calendar.v1",
    "flamel.google-drive.v1",
    "flamel.notion.v1",
    "flamel.slack.v1",
    "flamel.weather.v1",
    "flamel.maps.v1",
    "flamel.home-assistant.v1",
];

const LIFESTYLE_SLUGS: &[&str] = &[
    "gmail",
    "outlook-mail",
    "google-calendar",
    "outlook-calendar",
    "google-drive",
    "notion",
    "slack",
    "weather",
    "maps",
    "home-assistant",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Read,
    Mutate,
}

#[derive(Clone, Copy, Debug)]
enum Pagination {
    None,
    Link,
    Header(&'static str),
    Body(&'static str),
}

#[derive(Clone, Copy, Debug)]
struct ToolContract {
    name: &'static str,
    description: &'static str,
    method: &'static str,
    path: &'static str,
    required: &'static [&'static str],
    operation: Operation,
    schedule_eligible: bool,
    pagination: Pagination,
}

#[derive(Clone, Copy, Debug)]
struct PackContract {
    slug: &'static str,
    title: &'static str,
    description: &'static str,
    category: &'static str,
    homepage: &'static str,
    base_url: &'static str,
    allow_credential_base_url: bool,
    auth_kind: &'static str,
    credential_kinds: &'static [&'static str],
    scopes: &'static [&'static str],
    icon: &'static str,
    accent: &'static str,
    tools: &'static [ToolContract],
    health_tool: &'static str,
}

const GITHUB_TOOLS: &[ToolContract] = &[
    read(
        "github.health",
        "Verify GitHub API identity",
        "GET",
        "/user",
        &[],
        false,
        Pagination::None,
    ),
    read(
        "github.repository.get",
        "Read repository metadata",
        "GET",
        "/repos/{owner}/{repo}",
        &["owner", "repo"],
        true,
        Pagination::None,
    ),
    read(
        "github.actions.runs",
        "List workflow runs",
        "GET",
        "/repos/{owner}/{repo}/actions/runs",
        &["owner", "repo"],
        true,
        Pagination::Link,
    ),
];
const GITLAB_TOOLS: &[ToolContract] = &[
    read(
        "gitlab.health",
        "Verify GitLab API identity",
        "GET",
        "/user",
        &[],
        false,
        Pagination::None,
    ),
    read(
        "gitlab.project.get",
        "Read project metadata",
        "GET",
        "/projects/{projectId}",
        &["projectId"],
        true,
        Pagination::None,
    ),
    read(
        "gitlab.pipelines.list",
        "List project pipelines",
        "GET",
        "/projects/{projectId}/pipelines",
        &["projectId"],
        true,
        Pagination::Header("x-next-page"),
    ),
];
const LINEAR_TOOLS: &[ToolContract] = &[
    read(
        "linear.health",
        "Verify Linear viewer identity",
        "POST",
        "/graphql",
        &["body"],
        false,
        Pagination::None,
    ),
    read(
        "linear.issues.query",
        "Query issues through reviewed GraphQL",
        "POST",
        "/graphql",
        &["body"],
        true,
        Pagination::Body("/data/pageInfo/endCursor"),
    ),
    mutate(
        "linear.issue.update",
        "Update one Linear issue",
        "POST",
        "/graphql",
        &["body", "idempotencyKey"],
    ),
];
const SENTRY_TOOLS: &[ToolContract] = &[
    read(
        "sentry.health",
        "List accessible Sentry organizations",
        "GET",
        "/api/0/organizations/",
        &[],
        false,
        Pagination::Link,
    ),
    read(
        "sentry.issues.list",
        "List organization issues",
        "GET",
        "/api/0/organizations/{organization}/issues/",
        &["organization"],
        true,
        Pagination::Link,
    ),
    mutate(
        "sentry.issue.update",
        "Update one Sentry issue",
        "PUT",
        "/api/0/issues/{issueId}/",
        &["issueId", "body", "idempotencyKey"],
    ),
];
const VERCEL_TOOLS: &[ToolContract] = &[
    read(
        "vercel.health",
        "Verify Vercel identity",
        "GET",
        "/v2/user",
        &[],
        false,
        Pagination::None,
    ),
    read(
        "vercel.projects.list",
        "List Vercel projects",
        "GET",
        "/v9/projects",
        &[],
        true,
        Pagination::Body("/pagination/next"),
    ),
    mutate(
        "vercel.deployment.create",
        "Create a Vercel deployment",
        "POST",
        "/v13/deployments",
        &["body", "idempotencyKey"],
    ),
];
const CLOUDFLARE_TOOLS: &[ToolContract] = &[
    read(
        "cloudflare.health",
        "Verify Cloudflare token",
        "GET",
        "/client/v4/user/tokens/verify",
        &[],
        false,
        Pagination::None,
    ),
    read(
        "cloudflare.zones.list",
        "List Cloudflare zones",
        "GET",
        "/client/v4/zones",
        &[],
        true,
        Pagination::Body("/result_info/page"),
    ),
    mutate(
        "cloudflare.cache.purge",
        "Purge a zone cache",
        "POST",
        "/client/v4/zones/{zoneId}/purge_cache",
        &["zoneId", "body", "idempotencyKey"],
    ),
];
const POSTHOG_TOOLS: &[ToolContract] = &[
    read(
        "posthog.health",
        "Verify PostHog project access",
        "GET",
        "/api/projects/{projectId}/",
        &["projectId"],
        false,
        Pagination::None,
    ),
    read(
        "posthog.insight.query",
        "Run a PostHog query",
        "POST",
        "/api/projects/{projectId}/query/",
        &["projectId", "body"],
        true,
        Pagination::Body("/next"),
    ),
    mutate(
        "posthog.annotation.create",
        "Create a PostHog annotation",
        "POST",
        "/api/projects/{projectId}/annotations/",
        &["projectId", "body", "idempotencyKey"],
    ),
];
const AWS_TOOLS: &[ToolContract] = &[
    read(
        "aws.health",
        "Verify AWS Resource Explorer access",
        "POST",
        "/Search",
        &["body"],
        false,
        Pagination::None,
    ),
    read(
        "aws.resources.search",
        "Search AWS Resource Explorer inventory",
        "POST",
        "/Search",
        &["body"],
        true,
        Pagination::Body("/NextToken"),
    ),
];
const KUBERNETES_TOOLS: &[ToolContract] = &[
    read(
        "kubernetes.health",
        "Read Kubernetes server version",
        "GET",
        "/version",
        &[],
        false,
        Pagination::None,
    ),
    read(
        "kubernetes.pods.list",
        "List namespace pods",
        "GET",
        "/api/v1/namespaces/{namespace}/pods",
        &["namespace"],
        true,
        Pagination::Body("/metadata/continue"),
    ),
    mutate(
        "kubernetes.deployment.patch",
        "Patch a Kubernetes deployment",
        "PATCH",
        "/apis/apps/v1/namespaces/{namespace}/deployments/{name}",
        &["namespace", "name", "body", "idempotencyKey"],
    ),
];
const GRAFANA_TOOLS: &[ToolContract] = &[
    read(
        "grafana.health",
        "Read Grafana health",
        "GET",
        "/api/health",
        &[],
        false,
        Pagination::None,
    ),
    read(
        "grafana.search",
        "Search Grafana dashboards and folders",
        "GET",
        "/api/search",
        &[],
        true,
        Pagination::None,
    ),
    mutate(
        "grafana.annotation.create",
        "Create a Grafana annotation",
        "POST",
        "/api/annotations",
        &["body", "idempotencyKey"],
    ),
];

const GMAIL_TOOLS: &[ToolContract] = &[
    read(
        "gmail.health",
        "Verify Gmail mailbox access",
        "GET",
        "/gmail/v1/users/me/profile",
        &[],
        false,
        Pagination::None,
    ),
    read(
        "gmail.messages.search",
        "Search Gmail messages for an inbox summary",
        "GET",
        "/gmail/v1/users/me/messages",
        &["q"],
        true,
        Pagination::Body("/nextPageToken"),
    ),
    read(
        "gmail.message.get",
        "Read one Gmail message for the current task",
        "GET",
        "/gmail/v1/users/me/messages/{messageId}",
        &["messageId"],
        false,
        Pagination::None,
    ),
    mutate(
        "gmail.message.send",
        "Send one Gmail message",
        "POST",
        "/gmail/v1/users/me/messages/send",
        &["body", "idempotencyKey"],
    ),
];
const OUTLOOK_MAIL_TOOLS: &[ToolContract] = &[
    read(
        "outlook-mail.health",
        "Verify Outlook Mail identity",
        "GET",
        "/v1.0/me",
        &[],
        false,
        Pagination::None,
    ),
    read(
        "outlook-mail.messages.search",
        "Search Outlook messages for an inbox summary",
        "GET",
        "/v1.0/me/messages",
        &["filter"],
        true,
        Pagination::Body("/@odata.nextLink"),
    ),
    mutate(
        "outlook-mail.message.send",
        "Send one Outlook message",
        "POST",
        "/v1.0/me/sendMail",
        &["body", "idempotencyKey"],
    ),
];
const GOOGLE_CALENDAR_TOOLS: &[ToolContract] = &[
    read(
        "google-calendar.health",
        "Verify Google Calendar access",
        "GET",
        "/calendar/v3/users/me/calendarList",
        &[],
        false,
        Pagination::Body("/nextPageToken"),
    ),
    read(
        "google-calendar.events.list",
        "List events in an explicit timezone-aware window",
        "GET",
        "/calendar/v3/calendars/{calendarId}/events",
        &["calendarId", "timeMin", "timeMax", "timeZone"],
        true,
        Pagination::Body("/nextPageToken"),
    ),
    mutate(
        "google-calendar.event.create",
        "Create one calendar event",
        "POST",
        "/calendar/v3/calendars/{calendarId}/events",
        &["calendarId", "timeZone", "body", "idempotencyKey"],
    ),
];
const OUTLOOK_CALENDAR_TOOLS: &[ToolContract] = &[
    read(
        "outlook-calendar.health",
        "Verify Outlook Calendar identity",
        "GET",
        "/v1.0/me/calendar",
        &[],
        false,
        Pagination::None,
    ),
    read(
        "outlook-calendar.events.list",
        "List events in an explicit timezone-aware window",
        "GET",
        "/v1.0/me/calendarView",
        &["startDateTime", "endDateTime", "timeZone"],
        true,
        Pagination::Body("/@odata.nextLink"),
    ),
    mutate(
        "outlook-calendar.event.create",
        "Create one Outlook calendar event",
        "POST",
        "/v1.0/me/events",
        &["timeZone", "body", "idempotencyKey"],
    ),
];
const GOOGLE_DRIVE_TOOLS: &[ToolContract] = &[
    read(
        "google-drive.health",
        "Verify Google Drive identity",
        "GET",
        "/drive/v3/about",
        &["fields"],
        false,
        Pagination::None,
    ),
    read(
        "google-drive.files.search",
        "Search Drive file metadata",
        "GET",
        "/drive/v3/files",
        &["q"],
        true,
        Pagination::Body("/nextPageToken"),
    ),
    read(
        "google-drive.file.export",
        "Read one requested Google document for the current task",
        "GET",
        "/drive/v3/files/{fileId}/export",
        &["fileId", "mimeType"],
        false,
        Pagination::None,
    ),
    mutate(
        "google-drive.file.create",
        "Create one Drive file",
        "POST",
        "/drive/v3/files",
        &["body", "idempotencyKey"],
    ),
];
const NOTION_TOOLS: &[ToolContract] = &[
    read(
        "notion.health",
        "Verify Notion identity",
        "GET",
        "/v1/users/me",
        &[],
        false,
        Pagination::None,
    ),
    read(
        "notion.search",
        "Search accessible Notion pages and databases",
        "POST",
        "/v1/search",
        &["body"],
        true,
        Pagination::Body("/next_cursor"),
    ),
    read(
        "notion.page.get",
        "Read one requested Notion page for the current task",
        "GET",
        "/v1/pages/{pageId}",
        &["pageId"],
        false,
        Pagination::None,
    ),
    mutate(
        "notion.page.create",
        "Create one Notion page",
        "POST",
        "/v1/pages",
        &["body", "idempotencyKey"],
    ),
];
const SLACK_TOOLS: &[ToolContract] = &[
    read(
        "slack.health",
        "Verify Slack identity",
        "POST",
        "/api/auth.test",
        &[],
        false,
        Pagination::None,
    ),
    read(
        "slack.channel.history",
        "Read a bounded channel window for a current-task summary",
        "GET",
        "/api/conversations.history",
        &["channel"],
        true,
        Pagination::Body("/response_metadata/next_cursor"),
    ),
    mutate(
        "slack.message.send",
        "Send one Slack message",
        "POST",
        "/api/chat.postMessage",
        &["body", "idempotencyKey"],
    ),
];
const WEATHER_TOOLS: &[ToolContract] = &[
    read(
        "weather.health",
        "Verify the weather service",
        "GET",
        "/v1/forecast",
        &["latitude", "longitude"],
        false,
        Pagination::None,
    ),
    read(
        "weather.forecast",
        "Read a forecast for an explicit location",
        "GET",
        "/v1/forecast",
        &["latitude", "longitude", "timeZone"],
        true,
        Pagination::None,
    ),
];
const MAPS_TOOLS: &[ToolContract] = &[
    read(
        "maps.health",
        "Verify Google Maps access",
        "GET",
        "/maps/api/geocode/json",
        &["address"],
        false,
        Pagination::None,
    ),
    read(
        "maps.place.search",
        "Geocode an explicit place query",
        "GET",
        "/maps/api/geocode/json",
        &["address"],
        true,
        Pagination::None,
    ),
    read(
        "maps.route.plan",
        "Plan a route between explicit endpoints",
        "GET",
        "/maps/api/directions/json",
        &["origin", "destination", "mode"],
        true,
        Pagination::None,
    ),
];
const HOME_ASSISTANT_TOOLS: &[ToolContract] = &[
    read(
        "home-assistant.health",
        "Verify Home Assistant access",
        "GET",
        "/api/",
        &[],
        false,
        Pagination::None,
    ),
    read(
        "home-assistant.states.list",
        "Read current home entity state for the current task",
        "GET",
        "/api/states",
        &[],
        true,
        Pagination::None,
    ),
    mutate(
        "home-assistant.service.call",
        "Call one Home Assistant service against an explicit target",
        "POST",
        "/api/services/{domain}/{service}",
        &["domain", "service", "body", "idempotencyKey"],
    ),
];

const fn read(
    name: &'static str,
    description: &'static str,
    method: &'static str,
    path: &'static str,
    required: &'static [&'static str],
    schedule_eligible: bool,
    pagination: Pagination,
) -> ToolContract {
    ToolContract {
        name,
        description,
        method,
        path,
        required,
        operation: Operation::Read,
        schedule_eligible,
        pagination,
    }
}

const fn mutate(
    name: &'static str,
    description: &'static str,
    method: &'static str,
    path: &'static str,
    required: &'static [&'static str],
) -> ToolContract {
    ToolContract {
        name,
        description,
        method,
        path,
        required,
        operation: Operation::Mutate,
        schedule_eligible: false,
        pagination: Pagination::None,
    }
}

const PACKS: &[PackContract] = &[
    PackContract {
        slug: "github",
        title: "GitHub",
        description: "Repository and Actions insight with all repository mutations delegated to canonical Flamel Git",
        category: "developer",
        homepage: "https://github.com",
        base_url: "https://api.github.com",
        allow_credential_base_url: true,
        auth_kind: "oauth2",
        credential_kinds: &["oauth2", "token"],
        scopes: &["repositories.read", "actions.read"],
        icon: "github",
        accent: "slate",
        tools: GITHUB_TOOLS,
        health_tool: "github.health",
    },
    PackContract {
        slug: "gitlab",
        title: "GitLab",
        description: "Project and pipeline insight with all repository mutations delegated to canonical Flamel Git",
        category: "developer",
        homepage: "https://gitlab.com",
        base_url: "https://gitlab.com/api/v4",
        allow_credential_base_url: true,
        auth_kind: "token",
        credential_kinds: &["token", "oauth2"],
        scopes: &["projects.read", "pipelines.read"],
        icon: "gitlab",
        accent: "orange",
        tools: GITLAB_TOOLS,
        health_tool: "gitlab.health",
    },
    PackContract {
        slug: "linear",
        title: "Linear",
        description: "Issue discovery and approved issue updates",
        category: "productivity",
        homepage: "https://linear.app",
        base_url: "https://api.linear.app",
        allow_credential_base_url: false,
        auth_kind: "oauth2",
        credential_kinds: &["oauth2", "token"],
        scopes: &["issues.read", "issues.write"],
        icon: "linear",
        accent: "violet",
        tools: LINEAR_TOOLS,
        health_tool: "linear.health",
    },
    PackContract {
        slug: "sentry",
        title: "Sentry",
        description: "Production issue discovery and approved issue state updates",
        category: "observability",
        homepage: "https://sentry.io",
        base_url: "https://sentry.io",
        allow_credential_base_url: true,
        auth_kind: "oauth2",
        credential_kinds: &["oauth2", "token"],
        scopes: &["issues.read", "projects.read", "issues.write"],
        icon: "sentry",
        accent: "purple",
        tools: SENTRY_TOOLS,
        health_tool: "sentry.health",
    },
    PackContract {
        slug: "vercel",
        title: "Vercel",
        description: "Project and deployment operations with deployment creation gated by approval",
        category: "devops",
        homepage: "https://vercel.com",
        base_url: "https://api.vercel.com",
        allow_credential_base_url: false,
        auth_kind: "token",
        credential_kinds: &["token", "oauth2"],
        scopes: &["projects.read", "deployments.read", "deployments.write"],
        icon: "triangle",
        accent: "slate",
        tools: VERCEL_TOOLS,
        health_tool: "vercel.health",
    },
    PackContract {
        slug: "cloudflare",
        title: "Cloudflare",
        description: "Zone inspection and separately approved cache mutation",
        category: "devops",
        homepage: "https://cloudflare.com",
        base_url: "https://api.cloudflare.com",
        allow_credential_base_url: false,
        auth_kind: "token",
        credential_kinds: &["token"],
        scopes: &["zones.read", "cache.purge"],
        icon: "cloud",
        accent: "orange",
        tools: CLOUDFLARE_TOOLS,
        health_tool: "cloudflare.health",
    },
    PackContract {
        slug: "posthog",
        title: "PostHog",
        description: "Product analytics queries and approved annotations",
        category: "analytics",
        homepage: "https://posthog.com",
        base_url: "https://us.posthog.com",
        allow_credential_base_url: true,
        auth_kind: "token",
        credential_kinds: &["token", "oauth2"],
        scopes: &["insights.read", "annotations.write"],
        icon: "chart",
        accent: "amber",
        tools: POSTHOG_TOOLS,
        health_tool: "posthog.health",
    },
    PackContract {
        slug: "aws",
        title: "AWS",
        description: "Read-only AWS Resource Explorer inventory using native SigV4 credentials",
        category: "cloud",
        homepage: "https://aws.amazon.com",
        base_url: "https://resource-explorer-2.us-east-1.amazonaws.com",
        allow_credential_base_url: false,
        auth_kind: "token",
        credential_kinds: &["aws_sigv4"],
        scopes: &["resources.read"],
        icon: "cloud",
        accent: "amber",
        tools: AWS_TOOLS,
        health_tool: "aws.health",
    },
    PackContract {
        slug: "kubernetes",
        title: "Kubernetes",
        description: "Cluster workload inspection and approved deployment patches",
        category: "infrastructure",
        homepage: "https://kubernetes.io",
        base_url: "https://kubernetes.invalid",
        allow_credential_base_url: true,
        auth_kind: "token",
        credential_kinds: &["kubernetes_token"],
        scopes: &["cluster.read", "deployments.write"],
        icon: "kubernetes",
        accent: "blue",
        tools: KUBERNETES_TOOLS,
        health_tool: "kubernetes.health",
    },
    PackContract {
        slug: "grafana",
        title: "Grafana",
        description: "Dashboard discovery and approved operational annotations",
        category: "observability",
        homepage: "https://grafana.com",
        base_url: "https://grafana.com",
        allow_credential_base_url: true,
        auth_kind: "token",
        credential_kinds: &["token", "oauth2"],
        scopes: &["observability.read", "annotations.write"],
        icon: "activity",
        accent: "orange",
        tools: GRAFANA_TOOLS,
        health_tool: "grafana.health",
    },
    PackContract {
        slug: "gmail",
        title: "Gmail",
        description: "Inbox summaries and explicitly approved message sending",
        category: "communication",
        homepage: "https://mail.google.com",
        base_url: "https://gmail.googleapis.com",
        allow_credential_base_url: false,
        auth_kind: "oauth2",
        credential_kinds: &["oauth2"],
        scopes: &["mail.metadata.read", "mail.send"],
        icon: "mail",
        accent: "red",
        tools: GMAIL_TOOLS,
        health_tool: "gmail.health",
    },
    PackContract {
        slug: "outlook-mail",
        title: "Outlook Mail",
        description: "Inbox summaries and explicitly approved Outlook sending",
        category: "communication",
        homepage: "https://outlook.office.com",
        base_url: "https://graph.microsoft.com",
        allow_credential_base_url: false,
        auth_kind: "oauth2",
        credential_kinds: &["oauth2"],
        scopes: &["mail.read", "mail.send"],
        icon: "mail",
        accent: "blue",
        tools: OUTLOOK_MAIL_TOOLS,
        health_tool: "outlook-mail.health",
    },
    PackContract {
        slug: "google-calendar",
        title: "Google Calendar",
        description: "Timezone-safe calendar summaries and approved event creation",
        category: "calendar",
        homepage: "https://calendar.google.com",
        base_url: "https://www.googleapis.com",
        allow_credential_base_url: false,
        auth_kind: "oauth2",
        credential_kinds: &["oauth2"],
        scopes: &["calendar.read", "calendar.events.write"],
        icon: "calendar",
        accent: "blue",
        tools: GOOGLE_CALENDAR_TOOLS,
        health_tool: "google-calendar.health",
    },
    PackContract {
        slug: "outlook-calendar",
        title: "Outlook Calendar",
        description: "Timezone-safe Outlook calendar summaries and approved event creation",
        category: "calendar",
        homepage: "https://outlook.office.com/calendar",
        base_url: "https://graph.microsoft.com",
        allow_credential_base_url: false,
        auth_kind: "oauth2",
        credential_kinds: &["oauth2"],
        scopes: &["calendar.read", "calendar.events.write"],
        icon: "calendar",
        accent: "blue",
        tools: OUTLOOK_CALENDAR_TOOLS,
        health_tool: "outlook-calendar.health",
    },
    PackContract {
        slug: "google-drive",
        title: "Google Drive",
        description: "Document lookup with current-task reads and approved file creation",
        category: "documents",
        homepage: "https://drive.google.com",
        base_url: "https://www.googleapis.com",
        allow_credential_base_url: false,
        auth_kind: "oauth2",
        credential_kinds: &["oauth2"],
        scopes: &[
            "drive.metadata.read",
            "drive.content.read",
            "drive.files.write",
        ],
        icon: "drive",
        accent: "green",
        tools: GOOGLE_DRIVE_TOOLS,
        health_tool: "google-drive.health",
    },
    PackContract {
        slug: "notion",
        title: "Notion",
        description: "Workspace lookup with current-task page reads and approved page creation",
        category: "documents",
        homepage: "https://notion.so",
        base_url: "https://api.notion.com",
        allow_credential_base_url: false,
        auth_kind: "token",
        credential_kinds: &["token", "oauth2"],
        scopes: &["pages.read", "pages.write"],
        icon: "notion",
        accent: "slate",
        tools: NOTION_TOOLS,
        health_tool: "notion.health",
    },
    PackContract {
        slug: "slack",
        title: "Slack",
        description: "Channel summaries and explicitly approved message sending",
        category: "communication",
        homepage: "https://slack.com",
        base_url: "https://slack.com",
        allow_credential_base_url: false,
        auth_kind: "oauth2",
        credential_kinds: &["oauth2", "token"],
        scopes: &["channels.history.read", "messages.write"],
        icon: "slack",
        accent: "violet",
        tools: SLACK_TOOLS,
        health_tool: "slack.health",
    },
    PackContract {
        slug: "weather",
        title: "Weather",
        description: "Read-only weather forecasts for explicit locations",
        category: "lifestyle",
        homepage: "https://open-meteo.com",
        base_url: "https://api.open-meteo.com",
        allow_credential_base_url: false,
        auth_kind: "none",
        credential_kinds: &[],
        scopes: &[],
        icon: "cloud-sun",
        accent: "blue",
        tools: WEATHER_TOOLS,
        health_tool: "weather.health",
    },
    PackContract {
        slug: "maps",
        title: "Maps and Travel",
        description: "Place lookup and route planning for explicit locations",
        category: "travel",
        homepage: "https://maps.google.com",
        base_url: "https://maps.googleapis.com",
        allow_credential_base_url: false,
        auth_kind: "token",
        credential_kinds: &["api_key"],
        scopes: &["maps.read"],
        icon: "map",
        accent: "green",
        tools: MAPS_TOOLS,
        health_tool: "maps.health",
    },
    PackContract {
        slug: "home-assistant",
        title: "Home Assistant",
        description: "Private home status with explicitly approved service controls",
        category: "home",
        homepage: "https://home-assistant.io",
        base_url: "https://home-assistant.invalid",
        allow_credential_base_url: true,
        auth_kind: "token",
        credential_kinds: &["token"],
        scopes: &["home.states.read", "home.services.control"],
        icon: "home",
        accent: "blue",
        tools: HOME_ASSISTANT_TOOLS,
        health_tool: "home-assistant.health",
    },
];

fn contract(adapter: &str) -> Result<&'static PackContract, String> {
    PACKS
        .iter()
        .find(|pack| adapter == format!("flamel.{}.v1", pack.slug))
        .ok_or_else(|| {
            format!("Native adapter '{adapter}' is not compiled into this Flamel release")
        })
}

fn tool_contract<'a>(pack: &'a PackContract, name: &str) -> Result<&'a ToolContract, String> {
    pack.tools
        .iter()
        .find(|tool| tool.name == name)
        .ok_or_else(|| {
            format!(
                "Tool '{name}' is not part of the reviewed {} contract",
                pack.title
            )
        })
}

fn tool_schema(tool: &ToolContract) -> Value {
    let mut properties = Map::new();
    for field in tool.required {
        properties.insert(
            (*field).to_owned(),
            if *field == "body" {
                json!({"type":"object"})
            } else {
                json!({"type":"string","minLength":1})
            },
        );
    }
    properties
        .entry("query")
        .or_insert_with(|| json!({"type":"object"}));
    properties
        .entry("cursor")
        .or_insert_with(|| json!({"type":"string"}));
    properties
        .entry("limit")
        .or_insert_with(|| json!({"type":"integer","minimum":1,"maximum":100}));
    json!({
        "type":"object",
        "properties":properties,
        "required":tool.required,
        "additionalProperties":false
    })
}

fn pack_tool(pack: &PackContract, tool: &ToolContract) -> CapabilityPackTool {
    CapabilityPackTool {
        name: tool.name.into(),
        description: tool.description.into(),
        input_schema: tool_schema(tool),
        output_schema: Some(json!({"type":"object"})),
        annotations: json!({
            "readOnlyHint":tool.operation == Operation::Read,
            "destructiveHint":tool.operation == Operation::Mutate,
            "idempotentHint":tool.operation == Operation::Read || tool.required.contains(&"idempotencyKey"),
            "operation":if tool.operation == Operation::Read {"read"} else {"mutate"},
            "scheduleEligible":tool.schedule_eligible,
            "pagination":pagination_label(tool.pagination),
            "payloadRetention":if LIFESTYLE_SLUGS.contains(&pack.slug) {"ephemeral"} else {"task"},
            "privacyClass":if LIFESTYLE_SLUGS.contains(&pack.slug) {"personal"} else {"operational"},
        }),
        enabled_by_default: tool.operation == Operation::Read,
    }
}

fn pagination_label(pagination: Pagination) -> &'static str {
    match pagination {
        Pagination::None => "none",
        Pagination::Link => "link",
        Pagination::Header(_) => "header",
        Pagination::Body(_) => "body",
    }
}

pub fn release_manifests() -> Vec<CapabilityPackManifest> {
    PACKS
        .iter()
        .map(|pack| {
            let mutation_tools = pack
                .tools
                .iter()
                .filter(|tool| tool.operation == Operation::Mutate)
                .map(|tool| tool.name.to_owned())
                .collect::<Vec<_>>();
            let boundary = if matches!(pack.slug, "github" | "gitlab") {
                " Repository changes, issue writes, pull or merge request writes, and repository transport must use Flamel's canonical git tool instead of this pack."
            } else if LIFESTYLE_SLUGS.contains(&pack.slug) {
                " Treat returned mail, messages, documents, locations, calendars, and home telemetry as current-task context only. Never call memory or create a durable brain fact from connector payloads unless the user explicitly asks to remember one bounded fact. Never create a recurring schedule unless the user explicitly configures one."
            } else {
                ""
            };
            CapabilityPackManifest {
                schema_version: 1,
                id: format!("flamel.{}.pack", pack.slug),
                slug: pack.slug.into(),
                version: Version::new(1, 0, 0),
                title: pack.title.into(),
                description: pack.description.into(),
                category: pack.category.into(),
                publisher: "Flamel".into(),
                homepage_url: Some(pack.homepage.into()),
                connectors: vec![CapabilityPackConnector {
                    id: "primary".into(),
                    transport: "native".into(),
                    primary: true,
                    remote_url: None,
                    native_adapter: Some(format!("flamel.{}.v1", pack.slug)),
                }],
                tools: pack.tools.iter().map(|tool| pack_tool(pack, tool)).collect(),
                events: vec![CapabilityPackEvent {
                    name: format!("{}.updated", pack.slug),
                    description: format!("Normalized {} change event", pack.title),
                    schema: json!({"type":"object"}),
                }],
                auth: CapabilityPackAuth {
                    kind: pack.auth_kind.into(),
                    minimum_scopes: pack.scopes.iter().map(|scope| (*scope).into()).collect(),
                    credential_kinds: pack
                        .credential_kinds
                        .iter()
                        .map(|kind| (*kind).into())
                        .collect(),
                },
                skills: vec![CapabilityPackSkill {
                    slug: "operator".into(),
                    title: format!("{} operator", pack.title),
                    description: format!("Operate the reviewed {} capability boundary", pack.title),
                    category: pack.category.into(),
                    instructions: format!(
                        "Use only the signed {} tools advertised by the active capability. Prefer read-only inspection and concise audit-friendly summaries. Never invent scopes, bypass a tool-enforced approval, or change the configured API host from tool arguments. User-supplied values may appear in tool input and output when the task requires them; keep provider-owned capability references inside the active tool boundary. Mutations require an explicit idempotency key and the declared Flamel approval flow. Scheduled workflows may call only tools marked scheduleEligible=true.{}",
                        pack.title, boundary
                    ),
                    triggers: vec![
                        pack.slug.into(),
                        pack.title.to_ascii_lowercase(),
                        format!("{} status", pack.slug),
                    ],
                }],
                ui: CapabilityPackUi {
                    icon: pack.icon.into(),
                    accent: pack.accent.into(),
                    summary: pack.description.into(),
                },
                approval: CapabilityPackApproval {
                    decision: if mutation_tools.is_empty() {
                        "allow".into()
                    } else {
                        "require_approval".into()
                    },
                    tools: mutation_tools,
                },
                schedule_eligible: pack.tools.iter().any(|tool| tool.schedule_eligible),
                health_check: CapabilityPackHealthCheck {
                    tool_name: pack.health_tool.into(),
                    interval_seconds: 300,
                },
                uninstall: CapabilityPackUninstall {
                    revoke_credentials: true,
                    disable_schedules: true,
                },
                contract: CapabilityPackContract {
                    required_connections: vec!["primary".into()],
                    required_capabilities: Vec::new(),
                    permissions: pack
                        .tools
                        .iter()
                        .map(|tool| tool.name.to_owned())
                        .collect(),
                    supported_surfaces: vec!["assistant".into(), "automation".into(), "mcp".into()],
                    minimum_flamel_version: Some(
                        Version::parse(env!("CARGO_PKG_VERSION"))
                            .expect("crate version is valid semver"),
                    ),
                    provenance_url: Some("https://flamel.dev/security/capability-packs".into()),
                    evaluation_fixtures: vec![CapabilityPackEvaluationFixture {
                        name: "health".into(),
                        tool_name: pack.health_tool.into(),
                        input: json!({}),
                        expected_output_schema: None,
                    }],
                },
            }
        })
        .collect()
}

pub fn release_payloads() -> Result<Vec<Value>, String> {
    release_manifests()
        .into_iter()
        .map(|manifest| serde_json::to_value(manifest).map_err(|error| error.to_string()))
        .collect()
}

fn advertised_tools(adapter: &str) -> Result<Vec<Value>, String> {
    let pack = contract(adapter)?;
    release_manifests()
        .into_iter()
        .find(|manifest| manifest.slug == pack.slug)
        .map(|manifest| {
            manifest
                .tools
                .into_iter()
                .filter_map(|tool| serde_json::to_value(tool).ok())
                .collect()
        })
        .ok_or_else(|| "Compiled manifest is missing".into())
}

#[derive(Clone, Debug)]
struct NativeCredential {
    access_token: Option<String>,
    base_url: Option<String>,
    aws_access_key_id: Option<String>,
    aws_secret_access_key: Option<String>,
    aws_session_token: Option<String>,
    aws_region: Option<String>,
    project_id: Option<String>,
    api_key: Option<String>,
}

fn credential_config(raw: &str) -> Result<NativeCredential, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(NativeCredential {
            access_token: None,
            base_url: None,
            aws_access_key_id: None,
            aws_secret_access_key: None,
            aws_session_token: None,
            aws_region: None,
            project_id: None,
            api_key: None,
        });
    }
    if !trimmed.starts_with('{') {
        return Ok(NativeCredential {
            access_token: Some(trimmed.into()),
            base_url: None,
            aws_access_key_id: None,
            aws_secret_access_key: None,
            aws_session_token: None,
            aws_region: None,
            project_id: None,
            api_key: None,
        });
    }
    let value: Value =
        serde_json::from_str(trimmed).map_err(|_| "Encrypted credential JSON is invalid")?;
    let string = |name: &str| {
        value
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    Ok(NativeCredential {
        access_token: string("accessToken").or_else(|| string("token")),
        base_url: string("baseUrl"),
        aws_access_key_id: string("accessKeyId"),
        aws_secret_access_key: string("secretAccessKey"),
        aws_session_token: string("sessionToken"),
        aws_region: string("region"),
        project_id: string("projectId"),
        api_key: string("apiKey"),
    })
}

pub fn bearer_token(raw: &str) -> Result<String, String> {
    credential_config(raw)?
        .access_token
        .ok_or_else(|| "Authentication required: access token is missing".into())
}

fn safe_base_url(pack: &PackContract, credential: &NativeCredential) -> Result<Url, String> {
    let aws_endpoint;
    let candidate = if pack.slug == "aws" {
        let region = credential.aws_region.as_deref().unwrap_or("us-east-1");
        if region.len() > 32
            || region.is_empty()
            || !region.chars().all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
            })
        {
            return Err("AWS credential region is invalid".into());
        }
        aws_endpoint = format!("https://resource-explorer-2.{region}.amazonaws.com");
        aws_endpoint.as_str()
    } else if pack.allow_credential_base_url {
        credential.base_url.as_deref().unwrap_or(pack.base_url)
    } else {
        pack.base_url
    };
    let mut url = Url::parse(candidate).map_err(|_| "Configured capability API URL is invalid")?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return Err("Configured capability API URL must be absolute HTTPS".into());
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn path_arguments(path: &str) -> HashSet<&str> {
    path.split('{')
        .skip(1)
        .filter_map(|part| part.split_once('}').map(|(name, _)| name))
        .collect()
}

fn interpolate_path(path: &str, arguments: &Map<String, Value>) -> Result<String, String> {
    let mut result = path.to_owned();
    for field in path_arguments(path) {
        let value = arguments
            .get(field)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Missing path argument '{field}'"))?;
        result = result.replace(&format!("{{{field}}}"), &urlencoding::encode(value));
    }
    Ok(result)
}

fn checked_absolute_cursor(
    base: &Url,
    expected_path: &str,
    pagination: Pagination,
    cursor: &str,
) -> Result<Option<Url>, String> {
    let Ok(mut candidate) = Url::parse(cursor) else {
        return Ok(None);
    };
    let allows_absolute = matches!(pagination, Pagination::Link)
        || matches!(pagination, Pagination::Body(pointer) if pointer.ends_with("nextLink"));
    if !allows_absolute {
        return Err("Absolute pagination cursor is not allowed for this reviewed tool".into());
    }
    if candidate.scheme() != "https"
        || candidate.host_str() != base.host_str()
        || candidate.port_or_known_default() != base.port_or_known_default()
        || !candidate.username().is_empty()
        || candidate.password().is_some()
        || candidate.path() != expected_path
    {
        return Err(
            "Absolute pagination cursor must preserve the reviewed HTTPS host and path".into(),
        );
    }
    candidate.set_fragment(None);
    Ok(Some(candidate))
}

fn build_url(
    pack: &PackContract,
    tool: &ToolContract,
    credential: &NativeCredential,
    arguments: &Map<String, Value>,
) -> Result<Url, String> {
    let mut base = safe_base_url(pack, credential)?;
    let base_path = base.path().trim_end_matches('/');
    let path = interpolate_path(tool.path, arguments)?;
    base.set_path(&format!("{base_path}{path}"));
    if let Some(cursor) = arguments.get("cursor").and_then(Value::as_str) {
        let expected_path = base.path().to_owned();
        if let Some(candidate) =
            checked_absolute_cursor(&base, &expected_path, tool.pagination, cursor)?
        {
            return Ok(candidate);
        }
    }
    {
        let mut query = base.query_pairs_mut();
        if let Some(values) = arguments.get("query").and_then(Value::as_object) {
            for (key, value) in values {
                if let Some(value) = value.as_str() {
                    query.append_pair(key, value);
                } else if value.is_number() || value.is_boolean() {
                    query.append_pair(key, &value.to_string());
                }
            }
        }
        let path_fields = path_arguments(tool.path);
        for field in tool.required {
            if path_fields.contains(field)
                || matches!(*field, "body" | "idempotencyKey")
                || (pack.slug == "outlook-calendar" && *field == "timeZone")
            {
                continue;
            }
            let Some(value) = arguments.get(*field).and_then(Value::as_str) else {
                continue;
            };
            let key = match (pack.slug, *field) {
                ("outlook-mail", "filter") => "$filter",
                ("weather", "timeZone") => "timezone",
                _ => *field,
            };
            query.append_pair(key, value);
        }
        if pack.slug == "outlook-mail" && tool.name == "outlook-mail.messages.search" {
            query.append_pair(
                "$select",
                "id,subject,from,receivedDateTime,isRead,importance,webLink",
            );
        }
        if let Some(cursor) = arguments.get("cursor").and_then(Value::as_str) {
            let cursor_key = match pack.slug {
                "gitlab" => "page",
                "aws" => "NextToken",
                "gmail" | "google-calendar" | "google-drive" => "pageToken",
                _ => "cursor",
            };
            query.append_pair(cursor_key, cursor);
        }
        if let Some(limit) = arguments.get("limit").and_then(Value::as_u64) {
            let limit_key = if pack.slug == "gitlab" {
                "per_page"
            } else {
                "limit"
            };
            query.append_pair(limit_key, &limit.min(100).to_string());
        }
    }
    Ok(base)
}

fn request_body(arguments: &Map<String, Value>) -> Option<Value> {
    arguments.get("body").cloned()
}

fn outlook_timezone_header(
    pack: &PackContract,
    arguments: &Map<String, Value>,
) -> Result<Option<String>, String> {
    if pack.slug != "outlook-calendar" {
        return Ok(None);
    }
    let timezone = arguments
        .get("timeZone")
        .and_then(Value::as_str)
        .ok_or("Outlook Calendar requires an explicit timeZone")?;
    if timezone.is_empty()
        || timezone.len() > 128
        || timezone
            .chars()
            .any(|character| character.is_control() || matches!(character, '"' | '\\'))
    {
        return Err("Outlook Calendar timeZone is invalid".into());
    }
    Ok(Some(format!("outlook.timezone=\"{timezone}\"")))
}

fn summary(pack: &PackContract, tool: &ToolContract, data: &Value) -> String {
    let count = data
        .as_array()
        .map(Vec::len)
        .or_else(|| data.get("items").and_then(Value::as_array).map(Vec::len))
        .or_else(|| data.get("results").and_then(Value::as_array).map(Vec::len));
    match count {
        Some(count) => format!("{} {} returned {count} item(s)", pack.title, tool.name),
        None => format!("{} {} completed", pack.title, tool.name),
    }
}

fn pointer_string(value: &Value, pointer: &str) -> Option<String> {
    value.pointer(pointer).and_then(|value| match value {
        Value::String(value) if !value.is_empty() => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    })
}

fn next_cursor(pagination: Pagination, headers: &HeaderMap, body: &Value) -> Option<String> {
    match pagination {
        Pagination::None => None,
        Pagination::Header(name) => headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        Pagination::Body(pointer) => pointer_string(body, pointer),
        Pagination::Link => headers
            .get("link")
            .and_then(|value| value.to_str().ok())
            .and_then(|link| {
                link.split(',')
                    .find(|part| part.contains("rel=\"next\""))
                    .and_then(|part| part.split('<').nth(1))
                    .and_then(|part| part.split('>').next())
                    .map(str::to_owned)
            }),
    }
}

fn typed_http_error(status: u16, headers: &HeaderMap, body: &Value) -> String {
    let code = match status {
        401 => "authentication_required",
        403 => "forbidden",
        404 => "not_found",
        409 => "conflict",
        429 => "rate_limited",
        500..=599 => "vendor_unavailable",
        _ => "vendor_rejected",
    };
    let message = body
        .pointer("/error/message")
        .or_else(|| body.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("Vendor request failed");
    let retry = headers
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    format!("capability_http[{code}] status={status} retry_after={retry}: {message}")
}

fn vendor_semantic_error(pack: &PackContract, body: &Value) -> Option<String> {
    if pack.slug != "slack" || body.get("ok").and_then(Value::as_bool) != Some(false) {
        return None;
    }
    let code = body
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("unknown_error");
    Some(format!("capability_vendor[slack_error]: {code}"))
}

fn hmac_sha256(key: &[u8], data: &str) -> Result<Vec<u8>, String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| "AWS signing key is invalid")?;
    mac.update(data.as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}

fn aws_headers(
    method: &str,
    url: &Url,
    body: &[u8],
    credential: &NativeCredential,
) -> Result<Vec<(String, String)>, String> {
    let access_key = credential
        .aws_access_key_id
        .as_deref()
        .ok_or("Authentication required: AWS accessKeyId is missing")?;
    let secret_key = credential
        .aws_secret_access_key
        .as_deref()
        .ok_or("Authentication required: AWS secretAccessKey is missing")?;
    let region = credential.aws_region.as_deref().unwrap_or("us-east-1");
    let service = "resource-explorer-2";
    let now = Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = now.format("%Y%m%d").to_string();
    let host = url.host_str().ok_or("AWS endpoint host is missing")?;
    let payload_hash = hex::encode(Sha256::digest(body));
    let mut canonical_headers = format!("host:{host}\nx-amz-date:{amz_date}\n");
    let mut signed_headers = "host;x-amz-date".to_owned();
    let mut headers = vec![("x-amz-date".into(), amz_date.clone())];
    if let Some(session_token) = &credential.aws_session_token {
        canonical_headers.push_str(&format!("x-amz-security-token:{session_token}\n"));
        signed_headers.push_str(";x-amz-security-token");
        headers.push(("x-amz-security-token".into(), session_token.clone()));
    }
    let canonical_query = url.query().unwrap_or("");
    let canonical_request = format!(
        "{method}\n{}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}",
        url.path()
    );
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );
    let date_key = hmac_sha256(format!("AWS4{secret_key}").as_bytes(), &date)?;
    let region_key = hmac_sha256(&date_key, region)?;
    let service_key = hmac_sha256(&region_key, service)?;
    let signing_key = hmac_sha256(&service_key, "aws4_request")?;
    let signature = hex::encode(hmac_sha256(&signing_key, &string_to_sign)?);
    headers.push((
        "authorization".into(),
        format!(
            "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
        ),
    ));
    Ok(headers)
}

async fn send(
    pack: &PackContract,
    tool: &ToolContract,
    arguments: &Map<String, Value>,
    raw_credential: &str,
    cancellation: &CancellationToken,
) -> Result<Value, String> {
    let credential = credential_config(raw_credential)?;
    let url = build_url(pack, tool, &credential, arguments)?;
    let method = Method::from_bytes(tool.method.as_bytes())
        .map_err(|_| "Reviewed adapter has an invalid HTTP method")?;
    let body_value = request_body(arguments);
    let body_bytes = body_value
        .as_ref()
        .map(serde_json::to_vec)
        .transpose()
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    let client = Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(format!("Flamel-Assistant/{}", crate::BUILD_VERSION))
        .build()
        .map_err(|error| format!("Could not create capability HTTP client: {error}"))?;
    let mut request = client.request(method, url);
    if pack.slug == "aws" {
        for (name, value) in aws_headers(
            tool.method,
            request
                .try_clone()
                .ok_or("Could not clone AWS request")?
                .build()
                .map_err(|error| error.to_string())?
                .url(),
            &body_bytes,
            &credential,
        )? {
            request = request.header(name, value);
        }
        request = request.header("content-type", "application/x-amz-json-1.1");
        request = request.header("x-amz-target", "AWSResourceExplorer2.Search");
    } else if pack.slug == "maps" {
        let api_key = credential
            .api_key
            .as_deref()
            .or(credential.access_token.as_deref())
            .ok_or("Authentication required: Google Maps apiKey is missing")?;
        request = request.header("x-goog-api-key", api_key);
    } else if pack.auth_kind != "none" {
        let token = credential
            .access_token
            .as_deref()
            .ok_or("Authentication required: access token is missing")?;
        request = request.bearer_auth(token);
        if pack.slug == "github" {
            request = request
                .header("accept", "application/vnd.github+json")
                .header("x-github-api-version", "2022-11-28");
        }
        if pack.slug == "gitlab" {
            request = request.header("private-token", token);
        }
    }
    if pack.slug == "notion" {
        request = request.header("notion-version", "2022-06-28");
    }
    if let Some(timezone) = outlook_timezone_header(pack, arguments)? {
        request = request.header("prefer", timezone);
    }
    if tool.operation == Operation::Mutate {
        let key = arguments
            .get("idempotencyKey")
            .and_then(Value::as_str)
            .ok_or("Mutating capability calls require idempotencyKey")?;
        request = request.header("idempotency-key", key);
    }
    if pack.slug == "kubernetes" && tool.method == "PATCH" {
        request = request.header("content-type", "application/merge-patch+json");
    }
    if !body_bytes.is_empty() {
        if pack.slug != "aws" && !(pack.slug == "kubernetes" && tool.method == "PATCH") {
            request = request.header("content-type", "application/json");
        }
        request = request.body(body_bytes);
    }
    let response = tokio::select! {
        _ = cancellation.cancelled() => return Err("Capability task cancelled".into()),
        result = request.send() => result.map_err(|error| format!("capability_http[transport]: {error}"))?,
    };
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    if response
        .content_length()
        .is_some_and(|length| length > RESPONSE_LIMIT as u64)
    {
        return Err("capability_http[response_too_large]: vendor response exceeded 2 MiB".into());
    }
    let bytes = tokio::select! {
        _ = cancellation.cancelled() => return Err("Capability task cancelled".into()),
        result = response.bytes() => result.map_err(|error| format!("capability_http[transport]: {error}"))?,
    };
    if bytes.len() > RESPONSE_LIMIT {
        return Err("capability_http[response_too_large]: vendor response exceeded 2 MiB".into());
    }
    let body = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"text":String::from_utf8_lossy(&bytes)}))
    };
    if !(200..300).contains(&status) {
        return Err(typed_http_error(status, &headers, &body));
    }
    if let Some(error) = vendor_semantic_error(pack, &body) {
        return Err(error);
    }
    let cursor = next_cursor(tool.pagination, &headers, &body);
    Ok(json!({
        "ok":true,
        "vendor":pack.slug,
        "tool":tool.name,
        "status":status,
        "data":body,
        "pagination":{"nextCursor":cursor},
        "summary":summary(pack, tool, &body),
    }))
}

fn health_arguments(
    pack: &PackContract,
    raw_credential: &str,
) -> Result<Map<String, Value>, String> {
    let mut arguments = Map::new();
    match pack.slug {
        "linear" => {
            arguments.insert(
                "body".into(),
                json!({"query":"query FlamelHealth { viewer { id } }"}),
            );
        }
        "aws" => {
            arguments.insert(
                "body".into(),
                json!({"QueryString":"service:ec2","MaxResults":1}),
            );
        }
        "posthog" => {
            let credential = credential_config(raw_credential)?;
            arguments.insert(
                "projectId".into(),
                Value::String(
                    credential.project_id.ok_or(
                        "PostHog credential JSON must include projectId for health checks",
                    )?,
                ),
            );
        }
        "google-drive" => {
            arguments.insert("fields".into(), Value::String("user".into()));
        }
        "weather" => {
            arguments.insert("latitude".into(), Value::String("0".into()));
            arguments.insert("longitude".into(), Value::String("0".into()));
        }
        "maps" => {
            arguments.insert("address".into(), Value::String("Flamel".into()));
        }
        _ => {}
    }
    Ok(arguments)
}

pub async fn run_native(
    adapter: &str,
    call: Option<(&str, &Value)>,
    credential: Option<&str>,
    cancellation: &CancellationToken,
) -> Result<CapabilityResult, String> {
    let started = Instant::now();
    let pack = contract(adapter)?;
    let tools = advertised_tools(adapter)?;
    let raw_credential = credential.unwrap_or("");
    if pack.auth_kind != "none" && raw_credential.trim().is_empty() {
        return Err("Authentication required: capability credential is missing".into());
    }
    let output = if let Some((name, arguments)) = call {
        let tool = tool_contract(pack, name)?;
        let arguments = arguments
            .as_object()
            .ok_or("Capability tool arguments must be a JSON object")?;
        Some(send(pack, tool, arguments, raw_credential, cancellation).await?)
    } else {
        let tool = tool_contract(pack, pack.health_tool)?;
        let arguments = health_arguments(pack, raw_credential)?;
        Some(send(pack, tool, &arguments, raw_credential, cancellation).await?)
    };
    Ok(CapabilityResult {
        tools,
        resources: vec![],
        prompts: vec![],
        output,
        latency_ms: started.elapsed().as_millis() as i64,
        process_id: None,
    })
}

#[derive(Clone, Debug)]
pub struct RefreshedCredential {
    pub value: String,
    pub access_token: String,
    pub expires_at: Option<String>,
    pub refreshed: bool,
}

fn public_https_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| "OAuth token endpoint is invalid")?;
    if url.scheme() != "https" {
        return Err("OAuth token endpoint must use HTTPS".into());
    }
    let host = url
        .host_str()
        .ok_or("OAuth token endpoint host is missing")?;
    let denied = host.eq_ignore_ascii_case("localhost")
        || host.ends_with(".local")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified());
    if denied {
        return Err("OAuth token endpoint may not target a local host".into());
    }
    Ok(url)
}

fn apply_refresh_payload(
    mut credential: Value,
    payload: &Value,
    now: DateTime<Utc>,
) -> Result<RefreshedCredential, String> {
    let object = credential
        .as_object_mut()
        .ok_or("OAuth credential must be a JSON object")?;
    let access_token = payload
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("OAuth refresh response did not contain access_token")?
        .to_owned();
    object.insert("accessToken".into(), Value::String(access_token.clone()));
    if let Some(refresh_token) = payload.get("refresh_token").and_then(Value::as_str) {
        object.insert("refreshToken".into(), Value::String(refresh_token.into()));
    }
    let expires_at = payload
        .get("expires_in")
        .and_then(Value::as_i64)
        .map(|seconds| (now + ChronoDuration::seconds(seconds.max(30))).to_rfc3339());
    if let Some(expires_at) = &expires_at {
        object.insert("expiresAt".into(), Value::String(expires_at.clone()));
    }
    Ok(RefreshedCredential {
        value: credential.to_string(),
        access_token,
        expires_at,
        refreshed: true,
    })
}

pub async fn refresh_credential_if_needed(
    raw: &str,
    cancellation: &CancellationToken,
) -> Result<RefreshedCredential, String> {
    let trimmed = raw.trim();
    if !trimmed.starts_with('{') {
        return Ok(RefreshedCredential {
            value: trimmed.into(),
            access_token: trimmed.into(),
            expires_at: None,
            refreshed: false,
        });
    }
    let credential: Value =
        serde_json::from_str(trimmed).map_err(|_| "Encrypted credential JSON is invalid")?;
    let access_token = credential
        .get("accessToken")
        .or_else(|| credential.get("token"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let expires_at = credential
        .get("expiresAt")
        .and_then(Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));
    let needs_refresh =
        expires_at.is_some_and(|expires| expires <= Utc::now() + ChronoDuration::seconds(30));
    if !needs_refresh {
        return Ok(RefreshedCredential {
            value: trimmed.into(),
            access_token,
            expires_at: expires_at.map(|value| value.to_rfc3339()),
            refreshed: false,
        });
    }
    let endpoint = credential
        .get("tokenEndpoint")
        .and_then(Value::as_str)
        .ok_or("Authentication expired and no OAuth tokenEndpoint is configured")?;
    let endpoint = public_https_url(endpoint)?;
    let refresh_token = credential
        .get("refreshToken")
        .and_then(Value::as_str)
        .ok_or("Authentication expired and no refreshToken is configured")?;
    let client_id = credential
        .get("clientId")
        .and_then(Value::as_str)
        .ok_or("Authentication expired and no OAuth clientId is configured")?;
    let mut form = HashMap::from([
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ]);
    if let Some(secret) = credential.get("clientSecret").and_then(Value::as_str) {
        form.insert("client_secret", secret);
    }
    let client = Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("Could not create OAuth client: {error}"))?;
    let response = tokio::select! {
        _ = cancellation.cancelled() => return Err("Capability task cancelled".into()),
        result = client.post(endpoint).form(&form).send() => result.map_err(|error| format!("OAuth refresh failed: {error}"))?,
    };
    let status = response.status();
    let payload: Value = response
        .json()
        .await
        .map_err(|error| format!("OAuth refresh response was invalid: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "OAuth refresh failed with HTTP {}",
            status.as_u16()
        ));
    }
    apply_refresh_payload(credential, &payload, Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[test]
    fn all_twenty_release_manifests_are_deterministic_and_native() {
        let first = serde_json::to_string(&release_payloads().unwrap()).unwrap();
        let second = serde_json::to_string(&release_payloads().unwrap()).unwrap();
        assert_eq!(first, second);
        let manifests = release_manifests();
        assert_eq!(manifests.len(), NATIVE_ADAPTER_IDS.len());
        let slugs = manifests
            .iter()
            .map(|manifest| manifest.slug.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(slugs.len(), NATIVE_ADAPTER_IDS.len());
        assert_eq!(
            slugs
                .iter()
                .filter(|slug| LIFESTYLE_SLUGS.contains(&**slug))
                .count(),
            LIFESTYLE_SLUGS.len()
        );
        for manifest in manifests {
            crate::db::capability_packs::validate_manifest(&manifest).unwrap();
            assert_eq!(manifest.publisher, "Flamel");
            assert_eq!(manifest.connectors.len(), 1);
            assert_eq!(manifest.connectors[0].transport, "native");
            assert!(
                NATIVE_ADAPTER_IDS
                    .contains(&manifest.connectors[0].native_adapter.as_deref().unwrap())
            );
            assert!(
                manifest
                    .tools
                    .iter()
                    .any(|tool| tool.name == manifest.health_check.tool_name)
            );
            let instructions = &manifest.skills[0].instructions;
            assert!(instructions.contains("User-supplied values may appear"));
            assert!(!instructions.contains("place secrets in output"));
        }
    }

    #[test]
    fn every_pack_has_health_read_and_scheduled_contracts_and_mutations_are_gated() {
        for manifest in release_manifests() {
            assert!(manifest.schedule_eligible);
            assert!(
                manifest
                    .tools
                    .iter()
                    .any(|tool| tool.annotations["scheduleEligible"] == true)
            );
            let mutations = manifest
                .tools
                .iter()
                .filter(|tool| tool.annotations["operation"] == "mutate")
                .map(|tool| tool.name.clone())
                .collect::<Vec<_>>();
            if mutations.is_empty() {
                assert!(matches!(
                    manifest.slug.as_str(),
                    "github" | "gitlab" | "aws" | "weather" | "maps"
                ));
                assert_eq!(manifest.approval.decision, "allow");
            } else {
                assert_eq!(manifest.approval.decision, "require_approval");
                assert_eq!(manifest.approval.tools, mutations);
                for name in mutations {
                    let tool = manifest
                        .tools
                        .iter()
                        .find(|tool| tool.name == name)
                        .unwrap();
                    assert!(
                        tool.input_schema["required"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|field| field == "idempotencyKey")
                    );
                    assert!(!tool.enabled_by_default);
                }
            }
        }
    }

    #[test]
    fn lifestyle_manifests_are_private_exact_scope_and_never_hardcode_daily_work() {
        let manifests = release_manifests()
            .into_iter()
            .filter(|manifest| LIFESTYLE_SLUGS.contains(&manifest.slug.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(manifests.len(), LIFESTYLE_SLUGS.len());

        for manifest in manifests {
            let (auth_kind, credential_kinds, scopes): (&str, &[&str], &[&str]) =
                match manifest.slug.as_str() {
                    "gmail" => ("oauth2", &["oauth2"], &["mail.metadata.read", "mail.send"]),
                    "outlook-mail" => ("oauth2", &["oauth2"], &["mail.read", "mail.send"]),
                    "google-calendar" | "outlook-calendar" => (
                        "oauth2",
                        &["oauth2"],
                        &["calendar.read", "calendar.events.write"],
                    ),
                    "google-drive" => (
                        "oauth2",
                        &["oauth2"],
                        &[
                            "drive.metadata.read",
                            "drive.content.read",
                            "drive.files.write",
                        ],
                    ),
                    "notion" => (
                        "token",
                        &["token", "oauth2"],
                        &["pages.read", "pages.write"],
                    ),
                    "slack" => (
                        "oauth2",
                        &["oauth2", "token"],
                        &["channels.history.read", "messages.write"],
                    ),
                    "weather" => ("none", &[], &[]),
                    "maps" => ("token", &["api_key"], &["maps.read"]),
                    "home-assistant" => (
                        "token",
                        &["token"],
                        &["home.states.read", "home.services.control"],
                    ),
                    slug => panic!("unexpected lifestyle pack {slug}"),
                };
            assert_eq!(manifest.auth.kind, auth_kind);
            assert_eq!(
                manifest.auth.credential_kinds,
                credential_kinds
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                manifest.auth.minimum_scopes,
                scopes
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect::<Vec<_>>()
            );
            assert!(manifest.uninstall.revoke_credentials);
            assert!(manifest.uninstall.disable_schedules);
            assert!(manifest.tools.iter().all(|tool| {
                tool.annotations["payloadRetention"] == "ephemeral"
                    && tool.annotations["privacyClass"] == "personal"
            }));
            let instructions = &manifest.skills[0].instructions;
            assert!(instructions.contains("current-task context only"));
            assert!(instructions.contains("explicitly asks to remember one bounded fact"));
            assert!(instructions.contains(
                "Never create a recurring schedule unless the user explicitly configures one"
            ));
            let serialized = serde_json::to_string(&manifest)
                .unwrap()
                .to_ascii_lowercase();
            for forbidden in ["\"cron\"", "every day", "daily at", "0 0 * * *"] {
                assert!(
                    !serialized.contains(forbidden),
                    "{} contains {forbidden}",
                    manifest.slug
                );
            }
        }
    }

    #[test]
    fn lifestyle_vendor_fixtures_preserve_timezones_and_reviewed_boundaries() {
        let oauth = credential_config("secret-oauth-token").unwrap();
        let query_value = |url: &Url, key: &str| {
            url.query_pairs()
                .find(|(candidate, _)| candidate == key)
                .map(|(_, value)| value.into_owned())
        };

        let gmail = contract("flamel.gmail.v1").unwrap();
        let gmail_tool = tool_contract(gmail, "gmail.messages.search").unwrap();
        let gmail_args = json!({"q":"is:unread","cursor":"next-page"});
        let gmail_url =
            build_url(gmail, gmail_tool, &oauth, gmail_args.as_object().unwrap()).unwrap();
        assert_eq!(gmail_url.host_str(), Some("gmail.googleapis.com"));
        assert_eq!(query_value(&gmail_url, "q").as_deref(), Some("is:unread"));
        assert_eq!(
            query_value(&gmail_url, "pageToken").as_deref(),
            Some("next-page")
        );

        let outlook_mail = contract("flamel.outlook-mail.v1").unwrap();
        let outlook_mail_tool =
            tool_contract(outlook_mail, "outlook-mail.messages.search").unwrap();
        let outlook_args = json!({
            "filter":"isRead eq false",
            "cursor":"https://graph.microsoft.com/v1.0/me/messages?$skiptoken=opaque"
        });
        let outlook_url = build_url(
            outlook_mail,
            outlook_mail_tool,
            &oauth,
            outlook_args.as_object().unwrap(),
        )
        .unwrap();
        assert_eq!(
            query_value(&outlook_url, "$skiptoken").as_deref(),
            Some("opaque")
        );
        for rejected in [
            "https://evil.example/v1.0/me/messages?$skiptoken=opaque",
            "https://graph.microsoft.com/v1.0/me/drive?$skiptoken=opaque",
        ] {
            let args = json!({"filter":"isRead eq false","cursor":rejected});
            assert!(
                build_url(
                    outlook_mail,
                    outlook_mail_tool,
                    &oauth,
                    args.as_object().unwrap()
                )
                .unwrap_err()
                .contains("reviewed HTTPS host and path")
            );
        }

        let google_calendar = contract("flamel.google-calendar.v1").unwrap();
        let google_calendar_tool =
            tool_contract(google_calendar, "google-calendar.events.list").unwrap();
        let start = "2026-03-29T00:30:00+00:00";
        let end = "2026-03-29T04:30:00+01:00";
        let calendar_args = json!({
            "calendarId":"primary",
            "timeMin":start,
            "timeMax":end,
            "timeZone":"Europe/London"
        });
        let calendar_url = build_url(
            google_calendar,
            google_calendar_tool,
            &oauth,
            calendar_args.as_object().unwrap(),
        )
        .unwrap();
        assert_eq!(
            query_value(&calendar_url, "timeMin").as_deref(),
            Some(start)
        );
        assert_eq!(query_value(&calendar_url, "timeMax").as_deref(), Some(end));
        assert_eq!(
            query_value(&calendar_url, "timeZone").as_deref(),
            Some("Europe/London")
        );

        let outlook_calendar = contract("flamel.outlook-calendar.v1").unwrap();
        let outlook_calendar_tool =
            tool_contract(outlook_calendar, "outlook-calendar.events.list").unwrap();
        let outlook_calendar_args = json!({
            "startDateTime":start,
            "endDateTime":end,
            "timeZone":"Pacific Standard Time"
        });
        let outlook_calendar_url = build_url(
            outlook_calendar,
            outlook_calendar_tool,
            &oauth,
            outlook_calendar_args.as_object().unwrap(),
        )
        .unwrap();
        assert_eq!(
            query_value(&outlook_calendar_url, "startDateTime").as_deref(),
            Some(start)
        );
        assert_eq!(
            outlook_timezone_header(outlook_calendar, outlook_calendar_args.as_object().unwrap())
                .unwrap()
                .as_deref(),
            Some("outlook.timezone=\"Pacific Standard Time\"")
        );

        let drive = contract("flamel.google-drive.v1").unwrap();
        let drive_tool = tool_contract(drive, "google-drive.file.export").unwrap();
        let drive_args = json!({"fileId":"doc-1","mimeType":"text/plain"});
        let drive_url =
            build_url(drive, drive_tool, &oauth, drive_args.as_object().unwrap()).unwrap();
        assert_eq!(drive_url.host_str(), Some("www.googleapis.com"));
        assert_eq!(
            query_value(&drive_url, "mimeType").as_deref(),
            Some("text/plain")
        );

        let notion = contract("flamel.notion.v1").unwrap();
        let notion_tool = tool_contract(notion, "notion.search").unwrap();
        let notion_args = json!({"body":{"query":"roadmap"}});
        let notion_url = build_url(
            notion,
            notion_tool,
            &oauth,
            notion_args.as_object().unwrap(),
        )
        .unwrap();
        assert_eq!(notion_url.host_str(), Some("api.notion.com"));

        let slack = contract("flamel.slack.v1").unwrap();
        let slack_tool = tool_contract(slack, "slack.channel.history").unwrap();
        let slack_args = json!({"channel":"C123"});
        let slack_url =
            build_url(slack, slack_tool, &oauth, slack_args.as_object().unwrap()).unwrap();
        assert_eq!(query_value(&slack_url, "channel").as_deref(), Some("C123"));
        assert_eq!(
            vendor_semantic_error(slack, &json!({"ok":false,"error":"missing_scope"})),
            Some("capability_vendor[slack_error]: missing_scope".into())
        );

        let weather = contract("flamel.weather.v1").unwrap();
        let weather_tool = tool_contract(weather, "weather.forecast").unwrap();
        let weather_args = json!({
            "latitude":"51.5",
            "longitude":"-0.1",
            "timeZone":"Europe/London"
        });
        let weather_url = build_url(
            weather,
            weather_tool,
            &credential_config("").unwrap(),
            weather_args.as_object().unwrap(),
        )
        .unwrap();
        assert_eq!(weather_url.host_str(), Some("api.open-meteo.com"));
        assert_eq!(
            query_value(&weather_url, "timezone").as_deref(),
            Some("Europe/London")
        );
        assert_eq!(health_arguments(weather, "").unwrap()["latitude"], "0");

        let maps = contract("flamel.maps.v1").unwrap();
        let maps_tool = tool_contract(maps, "maps.route.plan").unwrap();
        let maps_credential =
            credential_config(r#"{"apiKey":"private-maps-key","baseUrl":"https://evil.example"}"#)
                .unwrap();
        let maps_args = json!({"origin":"London","destination":"Paris","mode":"transit"});
        let maps_url = build_url(
            maps,
            maps_tool,
            &maps_credential,
            maps_args.as_object().unwrap(),
        )
        .unwrap();
        assert_eq!(maps_url.host_str(), Some("maps.googleapis.com"));
        assert!(!maps_url.as_str().contains("private-maps-key"));

        let home = contract("flamel.home-assistant.v1").unwrap();
        let home_tool = tool_contract(home, "home-assistant.states.list").unwrap();
        let home_credential = credential_config(
            r#"{"accessToken":"private-home-token","baseUrl":"https://home.example"}"#,
        )
        .unwrap();
        let home_url = build_url(
            home,
            home_tool,
            &home_credential,
            json!({}).as_object().unwrap(),
        )
        .unwrap();
        assert_eq!(home_url.host_str(), Some("home.example"));
        assert!(!home_url.as_str().contains("private-home-token"));
    }

    #[test]
    fn vendor_fixture_paths_pagination_and_hosts_are_normalized() {
        let credential = credential_config(
            r#"{"accessToken":"secret","baseUrl":"https://gitlab.example/api/v4"}"#,
        )
        .unwrap();
        let gitlab = contract("flamel.gitlab.v1").unwrap();
        let tool = tool_contract(gitlab, "gitlab.pipelines.list").unwrap();
        let args = json!({"projectId":"group/project","limit":25,"cursor":"2"});
        let url = build_url(gitlab, tool, &credential, args.as_object().unwrap()).unwrap();
        assert_eq!(url.host_str(), Some("gitlab.example"));
        assert!(url.path().contains("group%2Fproject"));
        assert!(url.query().unwrap().contains("per_page=25"));
        assert_eq!(
            next_cursor(
                tool.pagination,
                &HeaderMap::from_iter([(
                    reqwest::header::HeaderName::from_static("x-next-page"),
                    reqwest::header::HeaderValue::from_static("3"),
                )]),
                &json!({})
            ),
            Some("3".into())
        );
        let fixed =
            credential_config(r#"{"accessToken":"secret","baseUrl":"https://evil.example"}"#)
                .unwrap();
        let cloudflare = contract("flamel.cloudflare.v1").unwrap();
        assert_eq!(
            safe_base_url(cloudflare, &fixed).unwrap().host_str(),
            Some("api.cloudflare.com")
        );
    }

    #[test]
    fn errors_and_pagination_have_stable_typed_contracts() {
        let headers = HeaderMap::from_iter([(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("42"),
        )]);
        let error = typed_http_error(429, &headers, &json!({"message":"slow down"}));
        assert!(error.contains("[rate_limited]"));
        assert!(error.contains("retry_after=42"));
        let link_headers = HeaderMap::from_iter([(
            reqwest::header::LINK,
            reqwest::header::HeaderValue::from_static(
                "<https://api.example/items?page=2>; rel=\"next\"",
            ),
        )]);
        assert_eq!(
            next_cursor(Pagination::Link, &link_headers, &json!({})),
            Some("https://api.example/items?page=2".into())
        );
    }

    #[test]
    fn oauth_refresh_rotation_preserves_encrypted_configuration_fields() {
        let original = json!({
            "accessToken":"old",
            "refreshToken":"refresh",
            "tokenEndpoint":"https://auth.example/token",
            "clientId":"client",
            "clientSecret":"secret",
            "baseUrl":"https://grafana.example",
            "expiresAt":"2020-01-01T00:00:00Z"
        });
        let refreshed = apply_refresh_payload(
            original,
            &json!({"access_token":"new","refresh_token":"rotated","expires_in":3600}),
            DateTime::parse_from_rfc3339("2026-08-19T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&refreshed.value).unwrap();
        assert_eq!(refreshed.access_token, "new");
        assert_eq!(value["refreshToken"], "rotated");
        assert_eq!(value["baseUrl"], "https://grafana.example");
        assert_eq!(value["clientSecret"], "secret");
        assert_eq!(
            refreshed.expires_at.as_deref(),
            Some("2026-08-19T01:00:00+00:00")
        );
    }

    #[test]
    fn github_and_gitlab_skills_keep_repository_mutations_on_flamel_git() {
        for manifest in release_manifests()
            .into_iter()
            .filter(|manifest| matches!(manifest.slug.as_str(), "github" | "gitlab"))
        {
            assert!(manifest.approval.tools.is_empty());
            assert!(
                !manifest
                    .tools
                    .iter()
                    .any(|tool| tool.annotations["operation"] == "mutate")
            );
            assert!(
                manifest.skills[0]
                    .instructions
                    .contains("canonical git tool")
            );
        }
    }

    #[test]
    fn aws_sigv4_fixture_never_exposes_secret_material() {
        let credential = NativeCredential {
            access_token: None,
            base_url: None,
            aws_access_key_id: Some("AKIDEXAMPLE".into()),
            aws_secret_access_key: Some("super-secret-value".into()),
            aws_session_token: Some(base64::engine::general_purpose::STANDARD.encode("session")),
            aws_region: Some("us-east-1".into()),
            project_id: None,
            api_key: None,
        };
        let url = Url::parse("https://resource-explorer-2.us-east-1.amazonaws.com/Search").unwrap();
        let headers = aws_headers(
            "POST",
            &url,
            br#"{"QueryString":"service:ec2"}"#,
            &credential,
        )
        .unwrap();
        let rendered = format!("{headers:?}");
        assert!(rendered.contains("AWS4-HMAC-SHA256"));
        assert!(!rendered.contains("super-secret-value"));
    }
}
