// Chat request/response types
export interface ChatRequest {
  message: string;
  requestId: string;
  workingDirectory: string;
  sessionId?: string;
  activePlanId?: string;
  title?: string;
  model?: string;
  ponytailEnabled?: boolean;
  permissionMode?: 'default' | 'acceptEdits' | 'bypassPermissions' | 'plan';
  mcpServers?: McpServerConfig[];
  images?: ImageAttachment[];
}

export interface ImageAttachment {
  data: string;       // base64
  mediaType: string;  // e.g. image/png
  name?: string;
}

export interface PermissionResponse {
  requestId: string;
  approved: boolean;
}

// Stream message types (NDJSON from backend to frontend)
export type StreamResponse =
  | { type: 'connected'; requestId: string }
  | { type: 'message'; message: SdkMessage }
  | { type: 'error'; error: string }
  | { type: 'done'; sessionId?: string; usage?: UsageSummary }
  | { type: 'ask_question'; requestId: string; sessionId?: string; runId?: string; status?: 'pending'; question: string; choices?: Array<string | QuestionChoice>; allowFreeText?: boolean; multiSelect?: boolean; previewMarkdown?: string }
  | { type: 'question_resolved'; requestId: string; sessionId?: string; runId?: string; status?: 'answered' }
  | { type: 'question_cancelled'; requestId: string; sessionId?: string; runId?: string; status?: 'cancelled' };

export interface QuestionChoice {
  label: string;
  description?: string;
}

export interface PendingQuestion {
  requestId: string;
  sessionId?: string;
  runId?: string;
  status?: "pending" | "answered" | "cancelled" | "consumed";
  question: string;
  choices?: Array<string | QuestionChoice>;
  allowFreeText?: boolean;
  multiSelect?: boolean;
  previewMarkdown?: string;
}

export interface UsageSummary {
  input_tokens: number;
  output_tokens: number;
  cache_read_input_tokens?: number;
  cache_creation_input_tokens?: number;
  total_cost_usd?: number;
}

// SDK message shapes (simplified for our use)
export type SdkMessage =
  | UserSdkMessage
  | AssistantSdkMessage
  | ToolResultSdkMessage
  | SystemSdkMessage
  | ResultSdkMessage;

export interface UserSdkMessage {
  type: 'user';
  message: { role: 'user'; content: ContentBlock[] };
}

export interface AssistantSdkMessage {
  type: 'assistant';
  message: {
    role: 'assistant';
    content: ContentBlock[];
    usage?: { input_tokens: number; output_tokens: number };
  };
}

export interface ToolResultSdkMessage {
  type: 'tool_result';
  tool_use_id: string;
  content: string;
  same_turn?: boolean;
  is_delta?: boolean;
}

export interface SystemSdkMessage {
  type: 'system';
  subtype: string;
  session_id?: string;
}

export interface ResultSdkMessage {
  type: 'result';
  subtype: 'success' | 'error_max_turns' | 'error_during_execution';
  session_id?: string;
  duration_ms?: number;
  num_turns?: number;
  usage?: UsageSummary;
  total_cost_usd?: number;
  is_error?: boolean;
  result?: string;
}

export type ContentBlock =
  | TextBlock
  | ThinkingBlock
  | ToolUseBlock
  | ToolResultBlock;

export interface TextBlock {
  type: 'text';
  text: string;
}

export interface ThinkingBlock {
  type: 'thinking';
  thinking: string;
  signature?: string;
}

export interface ToolUseBlock {
  type: 'tool_use';
  id: string;
  name: string;
  input: Record<string, unknown>;
}

export interface ToolResultBlock {
  type: 'tool_result';
  tool_use_id: string;
  content: string | ContentBlock[];
  is_error?: boolean;
  same_turn?: boolean;
  is_delta?: boolean;
}

// Session metadata
export interface SessionWorkspace {
  sourcePath: string;
  projectPath: string;
  branch?: string | null;
  baseSha?: string | null;
  baseBranch?: string | null;
  isolated: boolean;
}

export interface SessionMeta {
  id: string;
  claudeSessionId?: string;
  projectPath: string;
  workspace?: SessionWorkspace;
  title?: string;
  createdAt: string;
  updatedAt: string;
  messageCount: number;
  pinned: boolean;
  model?: string;
  inputTokens: number;
  outputTokens: number;
  isSessionWarm?: boolean;
  requiresResume?: boolean;
  sessionOwnerKind?: string | null;
  orchestrationId?: string | null;
  orchestrationRunId?: string | null;
  orchestrationRole?: string | null;
  activePlanId?: string | null;
  requestedModel: string;
  requestedReasoningEffort: string;
  resolvedReasoningEffort?: string | null;
  routingReason?: string | null;
}

export interface SessionBootstrap {
  session: SessionMeta;
  activePlan?: Plan | null;
  messages: Array<{
    type: string;
    uuid?: string;
    message: unknown;
  }>;
  binding: {
    sessionId: string;
    streamSessionId: string;
    activeRequestId?: string | null;
    activePlanId?: string | null;
    realtimeSequence: number;
    updatedAt: string;
  };
}

export type OrchestrationTargetKind = 'server' | 'local' | 'runner';
export type OrchestrationScheduleKind = 'manual' | 'interval' | 'event';
export type OrchestrationRunStatus = 'queued' | 'starting' | 'running' | 'waiting' | 'sleeping' | 'paused' | 'succeeded' | 'failed' | 'blocked' | 'cancelled';
export type OrchestrationProgressStage = 'checking' | 'planning' | 'working' | 'verifying' | 'needs_you' | 'finished';
export type OrchestrationReadinessStatus = 'ready' | 'warning' | 'blocked';
export type OrchestrationCampaignKind = 'campaign';
export type OrchestrationOperatingMode = 'advisory' | 'supervised_autonomous' | 'bounded_autonomous' | 'fully_autonomous';
export type OrchestrationGovernanceProfile = 'standard' | 'approval_gated' | 'release_strict' | 'research_readonly';
export type OrchestrationCampaignPhase =
  | 'define_goal'
  | 'select_target'
  | 'seed_team'
  | 'capability_check'
  | 'assign_workstreams'
  | 'collect_reports'
  | 'verify_evidence'
  | 'record_decision'
  | 'finalize_or_block';

export interface OrchestrationOperatingModeProfile {
  id: OrchestrationOperatingMode;
  label: string;
  summary: string;
  approval: 'human_required' | 'policy_breakpoints' | 'manager_directed';
  permissions: string;
  toolAccess: string;
  cadence: string;
  spend: string;
  escalation: string;
  teammateExecution: 'recommend_only' | 'supervised_work' | 'bounded_parallel_work' | 'continuous_parallel_work';
}

export const ORCHESTRATION_OPERATING_MODE_PROFILES: Record<OrchestrationOperatingMode, OrchestrationOperatingModeProfile> = {
  advisory: {
    id: 'advisory',
    label: 'Advisory',
    summary: 'Analyze, plan, and recommend work for a human operator to approve and execute.',
    approval: 'human_required',
    permissions: 'Read-oriented by default; implementation requires explicit operator approval.',
    toolAccess: 'Research, code navigation, docs, browser, and reporting tools; write/git/release tools stay gated.',
    cadence: 'Manual review cycles only.',
    spend: 'Minimal autonomous spend; stop before launching long-running teammate work.',
    escalation: 'Ask the operator before any state-changing action, external publication, or budget expansion.',
    teammateExecution: 'recommend_only',
  },
  supervised_autonomous: {
    id: 'supervised_autonomous',
    label: 'Supervised autonomous',
    summary: 'Let teammates do real work while destructive, external, credential, and release exceptions are surfaced for operator review.',
    approval: 'policy_breakpoints',
    permissions: 'Chat-grade Flamel MCP and project work are available for the signed-in user; sensitive exceptions are audited and surfaced.',
    toolAccess: 'Implementation, verification, browser, and local project tools match normal chat access inside the user workspace.',
    cadence: 'Coordinator runs supervised cycles and surfaces real blockers and review breakpoints.',
    spend: 'Respect definition budgets and pause before materially exceeding expected run cost.',
    escalation: 'Escalate real blockers, missing credentials, ambiguous requirements, and production-impacting changes.',
    teammateExecution: 'supervised_work',
  },
  bounded_autonomous: {
    id: 'bounded_autonomous',
    label: 'Bounded autonomous',
    summary: 'Run an autonomous team inside explicit project, budget, and completion boundaries.',
    approval: 'policy_breakpoints',
    permissions: 'Scoped writes, tests, browser automation, and git prep are allowed; publish/merge/release remain governed.',
    toolAccess: 'Parallel teammate tools are available inside project scope with audit and evidence records.',
    cadence: 'Coordinator may run repeated cycles, checkpoint, sleep, and resume until stop conditions are met.',
    spend: 'Budgets are enforced from max runtime, token, and cost settings when present.',
    escalation: 'Escalate stale work, failed checks, budget pressure, scope expansion, and policy conflicts.',
    teammateExecution: 'bounded_parallel_work',
  },
  fully_autonomous: {
    id: 'fully_autonomous',
    label: 'Fully autonomous',
    summary: 'Continuously drive a campaign with maximum delegation while preserving audit and budget controls.',
    approval: 'manager_directed',
    permissions: 'Broad project-scoped implementation and operational actions are allowed subject to global policy.',
    toolAccess: 'Coordinator can use parallel teammate workstreams, project tools, browser automation, and forge workflows.',
    cadence: 'Long-running cycles may continue across wakeups until completion, cancellation, or budget/policy stop.',
    spend: 'Use configured quotas as hard bounds and record budget posture every cycle.',
    escalation: 'Escalate only for blocked external dependencies, exhausted budgets, failed governance checks, or explicit stop conditions.',
    teammateExecution: 'continuous_parallel_work',
  },
};

export const ORCHESTRATION_ENTERPRISE_CONTRACT = {
  kind: 'campaign' as OrchestrationCampaignKind,
  coordinatorIsManagerOnly: true,
  substantiveWorkRequiresWorkerAssignment: true,
  finalizationRequiresWorkerEvidence: true,
  minimumIndependentWorkerReportsForEnterpriseSuccess: 0,
  workerEvidenceRequiredWhenDelegated: true,
  operatingModes: ['advisory', 'supervised_autonomous', 'bounded_autonomous', 'fully_autonomous'] as OrchestrationOperatingMode[],
  operatingModeProfiles: ORCHESTRATION_OPERATING_MODE_PROFILES,
  enterprisePersonas: ['operator', 'team_designer', 'security_admin', 'compliance_owner', 'engineering_manager', 'executive_sponsor', 'auditor'],
};

export interface OrchestrationTeamRole {
  name: string;
  subagent_type: string;
  responsibility: string;
  instructions?: string | null;
  owner?: string | null;
  risk_level?: 'low' | 'medium' | 'high' | 'critical' | string | null;
  approval_status?: 'draft' | 'review' | 'approved' | 'deprecated' | string | null;
  model?: string | null;
  permission_mode?: string | null;
  tools?: string[] | null;
  version?: string | null;
  lifecycle_stage?: 'draft' | 'staging' | 'production' | 'retired' | string | null;
  promotion_status?: 'not_requested' | 'requested' | 'approved' | 'rejected' | string | null;
  eval_status?: 'not_run' | 'passing' | 'failing' | 'waived' | string | null;
  eval_score?: number | null;
  regression_status?: 'not_run' | 'passing' | 'failing' | 'waived' | string | null;
  rollback_version?: string | null;
  persona_hash?: string | null;
}

export interface OrchestrationTeamTemplateMetadata {
  description?: string;
  owner?: string;
  version?: string;
  approval_status?: 'draft' | 'review' | 'approved' | 'deprecated' | string;
  risk_level?: 'low' | 'medium' | 'high' | 'critical' | string;
  visibility?: 'private' | 'workspace' | 'catalog' | string;
  tags?: string[];
  default_model?: string;
  default_permission_mode?: string;
  catalog_kind?: 'custom' | 'built_in' | 'imported' | string;
  lifecycle_stage?: 'draft' | 'staging' | 'production' | 'retired' | string;
  promotion_status?: 'not_requested' | 'requested' | 'approved' | 'rejected' | string;
  eval_status?: 'not_run' | 'passing' | 'failing' | 'waived' | string;
  eval_score?: number | null;
  eval_notes?: string;
  regression_status?: 'not_run' | 'passing' | 'failing' | 'waived' | string;
  rollback_version?: string;
  production_version?: string;
  staged_version?: string;
  persona_manifest?: any;
}

export interface OrchestrationTeamTemplate {
  id: string;
  user_id: string;
  name: string;
  roles: OrchestrationTeamRole[];
  metadata?: OrchestrationTeamTemplateMetadata | any;
  created_at: string;
  updated_at: string;
}

export interface CreateOrchestrationTeamTemplateRequest {
  name: string;
  roles: OrchestrationTeamRole[];
  metadata?: OrchestrationTeamTemplateMetadata | any;
}

export type UpdateOrchestrationTeamTemplateRequest = Partial<CreateOrchestrationTeamTemplateRequest>;

export interface OrchestrationDataPolicy {
  classification?: 'public' | 'internal' | 'confidential' | 'restricted' | string;
  residency?: string;
  allowed_read_scopes?: string[];
  allowed_write_scopes?: string[];
  retention_days?: number | null;
  transmission?: 'workspace_only' | 'approval_required' | 'allowed' | string;
  redact_secrets?: boolean;
  mask_pii?: boolean;
  allow_external_exports?: boolean;
  allow_credential_material?: boolean;
  export_controls?: 'deny' | 'approval_required' | 'allowed' | string;
}

export interface OrchestrationProjectSource {
  kind?: 'known_project' | 'local_path' | 'repository' | 'manual' | string;
  label?: string;
  local_path?: string;
  git_account_id?: string;
  provider?: 'github' | 'gitea' | string;
  account_label?: string;
  repo_id?: string;
  repo_name?: string;
  repo_full_name?: string;
  default_branch?: string;
  clone_url?: string;
  remote_url?: string;
  selected_at?: string;
}

export interface OrchestrationConnector {
  id: string;
  name: string;
  connector_kind?: 'mcp' | 'forge' | 'internal_api' | 'issue_tracker' | 'docs' | 'ci' | 'cloud' | 'chat' | 'knowledge_base' | string;
  provider?: string;
  owner?: string;
  risk_level?: 'low' | 'medium' | 'high' | 'critical' | string;
  approval_status?: 'approved' | 'requires_approval' | 'draft' | 'disabled' | string;
  enabled?: boolean;
  capabilities?: string[];
  allowed_tools?: string[];
  rate_limit?: string;
  audit_hook?: string;
  description?: string;
  metadata?: Record<string, unknown>;
}

export interface OrchestrationDefinition {
  id: string;
  user_id: string;
  name: string;
  goal: string;
  project_path: string;
  project_source?: OrchestrationProjectSource | any;
  target_kind: OrchestrationTargetKind;
  runner_id?: string | null;
  model: string;
  reasoning_effort?: string | null;
  permission_mode: string;
  team_template: string;
  team_config?: OrchestrationTeamRole[] | null;
  campaign_kind?: OrchestrationCampaignKind | string;
  operating_mode?: OrchestrationOperatingMode | string;
  governance_profile?: OrchestrationGovernanceProfile | string;
  campaign_phase?: OrchestrationCampaignPhase | string;
  scope?: any;
  success_criteria?: any;
  data_policy?: OrchestrationDataPolicy | any;
  connector_registry?: OrchestrationConnector[] | any;
  schedule_kind: OrchestrationScheduleKind;
  interval_secs?: number | null;
  enabled: boolean;
  max_runtime_secs?: number | null;
  max_tokens?: number | null;
  max_cost_usd?: number | null;
  checkpoint_policy: 'automatic' | 'manual' | 'off' | string;
  completion_condition?: string | null;
  last_run_at?: string | null;
  next_run_at?: string | null;
  run_count: number;
  created_at: string;
  updated_at: string;
}

export interface OrchestrationRun {
  id: string;
  orchestration_id: string;
  user_id: string;
  status: OrchestrationRunStatus;
  target_kind: OrchestrationTargetKind;
  runner_id?: string | null;
  coordinator_session_id?: string | null;
  coordinator_request_id?: string | null;
  lease_owner?: string | null;
  lease_expires_at?: string | null;
  last_heartbeat_at?: string | null;
  checkpoint_count: number;
  started_at: string;
  finished_at?: string | null;
  error?: string | null;
  summary?: string | null;
}

export interface OrchestrationReadinessCheck {
  id: string;
  label: string;
  status: 'pass' | 'warning' | 'block';
  message: string;
  fix?: string | null;
}

export interface OrchestrationRunProgress {
  stage: OrchestrationProgressStage;
  headline: string;
  detail: string;
  needs_attention: boolean;
  active_work: number;
  completed_work: number;
  total_work: number;
  updated_at: string;
}

export interface PrepareOrchestrationRequest {
  goal: string;
  project_path: string;
  project_source?: OrchestrationProjectSource | any;
  preferred_model?: string | null;
  preferred_reasoning_effort?: string | null;
  target_kind?: OrchestrationTargetKind;
  runner_id?: string | null;
}

export interface OrchestrationPreparation {
  draft: CreateOrchestrationRequest;
  review: {
    title: string;
    summary: string;
    project_label: string;
    target_label: string;
    model_label: string;
    team_label: string;
    permission_label: string;
  };
  readiness: {
    status: OrchestrationReadinessStatus;
    checks: OrchestrationReadinessCheck[];
  };
}

export interface LaunchOrchestrationRequest extends PrepareOrchestrationRequest {
  name?: string;
}


export interface OrchestrationCampaignState {
  status: OrchestrationRunStatus | string;
  phase?: OrchestrationCampaignPhase | string;
  schedule_kind?: OrchestrationScheduleKind | string;
  interval_secs?: number | null;
  enabled?: boolean;
  cycle_count: number;
  sleep_count: number;
  wake_count: number;
  next_wake_at?: string | null;
  wake_due?: boolean;
  last_run_at?: string | null;
  definition_next_run_at?: string | null;
  last_heartbeat_at?: string | null;
  lease_expires_at?: string | null;
  checkpoint_count: number;
  latest_checkpoint?: { id: string; label: string; created_at: string } | null;
  open_tasks: number;
  open_assignments: number;
  worker_reports: number;
  summary?: string | null;
  stop_conditions?: {
    completion_condition?: string | null;
    max_runtime_secs?: number | null;
    max_tokens?: number | null;
    max_cost_usd?: number | null;
    checkpoint_policy?: string;
  };
}


export interface OrchestrationAgent {
  id: string;
  run_id: string;
  name: string;
  role: string;
  model: string;
  status: string;
  started_at?: string | null;
  finished_at?: string | null;
  last_event?: string | null;
}

export interface OrchestrationAccessGrant {
  id: string;
  run_id: string;
  agent_id: string;
  assignment_id?: string | null;
  principal_kind: string;
  principal_id: string;
  capability: string;
  scope_kind: string;
  scope_value: string;
  grant_status: string;
  granted_by: string;
  rationale?: string | null;
  expires_at?: string | null;
  created_at: string;
  updated_at: string;
}

export interface OrchestrationTask {
  id: string;
  run_id: string;
  title: string;
  status: string;
  owner_agent_id?: string | null;
  position: number;
  evidence?: string | null;
  created_at: string;
  updated_at: string;
}

export interface OrchestrationAssignment {
  id: string;
  run_id: string;
  agent_id?: string | null;
  role_name: string;
  role_type: string;
  title: string;
  instructions?: string | null;
  status: string;
  priority: number;
  child_session_id?: string | null;
  child_run_id?: string | null;
  report_summary?: string | null;
  evidence?: string | null;
  created_at: string;
  updated_at: string;
  started_at?: string | null;
  finished_at?: string | null;
}

export interface OrchestrationReport {
  id: string;
  run_id: string;
  assignment_id?: string | null;
  agent_id?: string | null;
  report_type: string;
  status: string;
  title: string;
  summary: string;
  evidence?: string | null;
  payload: any;
  created_at: string;
}

export interface OrchestrationDecision {
  id: string;
  run_id: string;
  assignment_id?: string | null;
  decision_type: string;
  title: string;
  rationale?: string | null;
  status: string;
  payload: any;
  created_at: string;
}

export interface OrchestrationEvent {
  id: number;
  run_id: string;
  event_type: string;
  message: string;
  payload: any;
  created_at: string;
}

export interface OrchestrationArtifact {
  id: string;
  run_id: string;
  kind: string;
  title: string;
  path?: string | null;
  url?: string | null;
  payload: any;
  created_at: string;
}

export interface OrchestrationCheckpoint {
  id: string;
  run_id: string;
  label: string;
  state: any;
  created_at: string;
}

export interface OrchestrationPolicyDecision {
  id: string;
  run_id: string;
  assignment_id?: string | null;
  policy_name: string;
  input: any;
  decision: 'allow' | 'deny' | 'approval_required' | string;
  rationale?: string | null;
  enforced_by: string;
  created_at: string;
}

export interface OrchestrationAuditRecord {
  id: string;
  run_id: string;
  actor_kind: string;
  actor_id?: string | null;
  action: string;
  resource_type?: string | null;
  resource_id?: string | null;
  outcome: string;
  payload: any;
  created_at: string;
}

export interface OrchestrationLedgerEntry {
  id: string;
  run_id: string;
  entry_type: string;
  subject: string;
  content: string;
  evidence?: string | null;
  source_event_id?: number | null;
  created_at: string;
}

export interface OrchestrationBudgetEntry {
  id: string;
  run_id: string;
  agent_id?: string | null;
  assignment_id?: string | null;
  metric: string;
  amount: number;
  source?: string | null;
  created_at: string;
}

export interface OrchestrationBudgetUsage {
  token_total: number;
  cost_total_usd: number;
  runtime_seconds: number;
  tool_events: number;
  compute_events: number;
  storage_items: number;
  storage_estimate_kb: number;
  budget_entries: number;
  model_spend_usd: number;
}

export interface OrchestrationBudgetLimit {
  id: string;
  label: string;
  metric: string;
  unit: string;
  used: number;
  limit?: number | null;
  remaining?: number | null;
  percent?: number | null;
  status: 'ok' | 'near_limit' | 'exceeded' | 'unbounded' | string;
  enforcement: 'hard' | 'monitoring' | 'unbounded' | string;
  scope?: string | null;
}

export interface OrchestrationBudgetAllocation {
  id: string;
  kind: 'agent' | 'assignment' | 'source' | 'project' | 'team' | 'campaign' | 'user' | string;
  label: string;
  scope_id?: string | null;
  tokens: number;
  cost_usd: number;
  runtime_seconds: number;
  tool_events: number;
  storage_estimate_kb: number;
  entries: number;
  share_percent?: number | null;
}

export interface OrchestrationBudgetAlert {
  id: string;
  severity: 'info' | 'warning' | 'critical' | string;
  category: string;
  title: string;
  summary: string;
  status: string;
  scope_kind?: string | null;
  scope_id?: string | null;
  created_at: string;
}

export interface OrchestrationBudgetForecast {
  elapsed_hours: number;
  token_burn_rate_per_hour: number;
  cost_burn_rate_per_hour: number;
  projected_tokens_next_24h: number;
  projected_cost_next_24h: number;
  projected_runtime_seconds_next_cycle: number;
  estimated_token_exhaustion_at?: string | null;
  estimated_cost_exhaustion_at?: string | null;
  confidence: 'low' | 'medium' | 'high' | string;
  basis: string;
}

export interface OrchestrationBudgetState {
  status: 'ok' | 'watch' | 'near_limit' | 'exceeded' | 'unbounded' | string;
  summary: string;
  updated_at: string;
  usage: OrchestrationBudgetUsage;
  limits: OrchestrationBudgetLimit[];
  allocations: OrchestrationBudgetAllocation[];
  alerts: OrchestrationBudgetAlert[];
  forecast: OrchestrationBudgetForecast;
  chargeback: Record<string, any>;
  quotas: Record<string, any>;
}

export interface OrchestrationTeamMessage {
  id: string;
  run_id: string;
  thread_id: string;
  sender_agent_id?: string | null;
  recipient_agent_id?: string | null;
  assignment_id?: string | null;
  message_kind: string;
  subject: string;
  body: string;
  payload: any;
  created_at: string;
}

export interface OrchestrationWorkstreamState {
  assignment: OrchestrationAssignment;
  agent?: OrchestrationAgent | null;
  phase: string;
  needs_attention: boolean;
  peer_review_requested: boolean;
  message_count: number;
  latest_report?: OrchestrationReport | null;
  latest_message?: OrchestrationTeamMessage | null;
  blockers: OrchestrationTeamMessage[];
  handoffs: OrchestrationTeamMessage[];
  last_activity_at?: string | null;
}

export interface OrchestrationTeamCollaborationState {
  workstreams: OrchestrationWorkstreamState[];
  messages: OrchestrationTeamMessage[];
  open_blockers: OrchestrationTeamMessage[];
  handoffs: OrchestrationTeamMessage[];
  review_requests: OrchestrationTeamMessage[];
  active_threads: number;
  latest_activity_at?: string | null;
  collaboration_contract: string;
}

export interface OrchestrationProcessStreamItem {
  id: string;
  kind: string;
  title: string;
  summary: string;
  source: string;
  status?: string | null;
  actor?: string | null;
  assignment_id?: string | null;
  created_at: string;
  payload?: any;
}

export interface OrchestrationWorkerProcessStream {
  id: string;
  kind: 'worker_stream' | string;
  title: string;
  role_name: string;
  role_type: string;
  status: string;
  agent_id?: string | null;
  agent_name?: string | null;
  agent_status?: string | null;
  last_activity_at?: string | null;
  assignment?: OrchestrationAssignment;
  agent?: OrchestrationAgent | null;
  latest_report?: OrchestrationReport | null;
  latest_message?: OrchestrationTeamMessage | null;
  messages: OrchestrationProcessStreamItem[];
  reports: OrchestrationProcessStreamItem[];
  tool_activity: OrchestrationProcessStreamItem[];
}

export interface OrchestrationProcessStreams {
  run_id: string;
  visibility_contract?: string;
  manager: {
    phase?: string;
    pending_review?: boolean;
    loop_state?: OrchestrationManagerLoopState | null;
    items: OrchestrationProcessStreamItem[];
  };
  workers: {
    items: OrchestrationWorkerProcessStream[];
    count: number;
    active: number;
  };
  tool_activity: OrchestrationProcessStreamItem[];
  timeline: OrchestrationProcessStreamItem[];
  blockers: OrchestrationProcessStreamItem[];
  artifacts: OrchestrationProcessStreamItem[];
  raw_team_collaboration?: OrchestrationTeamCollaborationState | null;
  counts?: Record<string, number>;
}

export interface OrchestrationProjectStateItem {
  id?: string | null;
  kind: string;
  title: string;
  summary: string;
  status?: string | null;
  evidence?: string | null;
  source?: string | null;
  created_at?: string | null;
}

export interface OrchestrationProjectBudgetState {
  token_total: number;
  cost_total: number;
  max_tokens?: number | null;
  max_cost_usd?: number | null;
  entries: OrchestrationProjectStateItem[];
}

export interface OrchestrationProjectState {
  goal: string;
  project_path: string;
  project_source?: OrchestrationProjectSource | any;
  connector_registry?: OrchestrationConnector[] | any;
  run_status: string;
  campaign_phase?: string | null;
  scope: any;
  success_criteria: any;
  completion_condition?: string | null;
  updated_at?: string | null;
  compact_summary: string;
  requirements: OrchestrationProjectStateItem[];
  assumptions: OrchestrationProjectStateItem[];
  decisions: OrchestrationProjectStateItem[];
  open_questions: OrchestrationProjectStateItem[];
  workstreams: Array<OrchestrationWorkstreamState | OrchestrationProjectStateItem>;
  risks: OrchestrationProjectStateItem[];
  dependencies: OrchestrationProjectStateItem[];
  artifacts: OrchestrationProjectStateItem[];
  verification_evidence: OrchestrationProjectStateItem[];
  approvals: OrchestrationProjectStateItem[];
  budget: OrchestrationProjectBudgetState;
  next_cycle_intent: OrchestrationProjectStateItem[];
  recent_activity: OrchestrationProjectStateItem[];
  ledger_entry_count: number;
  source_counts: Record<string, number>;
  campaign_state?: OrchestrationCampaignState | null;
  manager_loop?: OrchestrationManagerLoopState | null;
}

export interface OrchestrationManagerLoopState {
  phase: string;
  pending_review: boolean;
  solo_risk: boolean;
  open_worker_assignments: number;
  completed_worker_reports: number;
  required_worker_reports: number;
  latest_report_at?: string | null;
  latest_review_at?: string | null;
  latest_delegation_at?: string | null;
  latest_next_action_at?: string | null;
  latest_report?: any | null;
  latest_directive?: any | null;
  latest_delegation?: any | null;
  latest_review?: any | null;
  latest_next_action?: any | null;
  anti_solo_invariant?: string;
}

export interface OrchestrationHealthSignal {
  id: string;
  severity: 'info' | 'warning' | 'critical' | 'success' | string;
  title: string;
  summary: string;
  source: string;
  status?: string | null;
  created_at?: string | null;
}

export interface OrchestrationTelemetryTrace {
  id: string;
  trace_type: string;
  title: string;
  summary: string;
  source: string;
  status?: string | null;
  created_at?: string | null;
  actor?: string | null;
}

export interface OrchestrationObservabilityState {
  health: 'idle' | 'active' | 'waiting' | 'sleeping' | 'paused' | 'degraded' | 'blocked' | 'failed' | 'complete' | string;
  health_summary: string;
  run_status: string;
  coordinator_status: string;
  heartbeat_state: 'not_started' | 'live' | 'expired' | 'terminal' | string;
  last_activity_at?: string | null;
  updated_at: string;
  counts: Record<string, number>;
  source_counts: Record<string, number>;
  activity_counts: Record<string, number>;
  signals: OrchestrationHealthSignal[];
  recent_traces: OrchestrationTelemetryTrace[];
}

export interface OrchestrationReliabilityIssue {
  id: string;
  severity: 'info' | 'warning' | 'critical' | string;
  category: string;
  title: string;
  summary: string;
  source: string;
  status?: string | null;
  target_kind?: string | null;
  target_id?: string | null;
  first_seen_at?: string | null;
  last_seen_at?: string | null;
}

export interface OrchestrationRecoveryAction {
  id: string;
  label: string;
  action: 'retry_coordinator' | 'quarantine_agent' | 'reassign_workstream' | 'rollback_checkpoint' | 'escalate' | 'sleep_next_cycle' | 'acknowledge' | string;
  severity: 'info' | 'warning' | 'critical' | string;
  target_kind?: string | null;
  target_id?: string | null;
  reason: string;
  status: 'recommended' | 'available' | 'blocked' | 'completed' | string;
  can_execute: boolean;
  automation_level: 'automatic' | 'operator' | 'approval_required' | string;
  created_at: string;
}

export interface OrchestrationCircuitBreakerState {
  id: string;
  name: string;
  state: 'closed' | 'half_open' | 'open' | string;
  severity: 'info' | 'warning' | 'critical' | string;
  summary: string;
  tripped_at?: string | null;
  reset_after_secs?: number | null;
}

export interface OrchestrationReliabilityState {
  posture: 'healthy' | 'degraded' | 'critical' | 'blocked' | 'failed' | 'sleeping' | 'paused' | 'waiting' | 'complete' | string;
  summary: string;
  updated_at: string;
  last_recovery_at?: string | null;
  retry_policy: Record<string, any>;
  counts: Record<string, number>;
  issues: OrchestrationReliabilityIssue[];
  recovery_actions: OrchestrationRecoveryAction[];
  circuit_breakers: OrchestrationCircuitBreakerState[];
}

export interface OrchestrationReliabilityActionRequest {
  action: string;
  target_kind?: string | null;
  target_id?: string | null;
  reason?: string | null;
}

export interface OrchestrationQualityGateCheck {
  id: string;
  gate: string;
  label: string;
  status: 'passed' | 'warning' | 'failed' | 'blocked' | 'pending' | 'not_applicable' | string;
  required: boolean;
  summary: string;
  evidence?: string | null;
  source?: string | null;
  updated_at?: string | null;
}

export interface OrchestrationReleaseReadiness {
  status: 'ready' | 'blocked' | 'warning' | 'pending' | string;
  summary: string;
  required_passed: number;
  required_total: number;
  optional_passed: number;
  optional_total: number;
  blockers: string[];
  warnings: string[];
}

export interface OrchestrationQualityGateState {
  status: 'ready' | 'blocked' | 'warning' | 'pending' | string;
  summary: string;
  updated_at: string;
  checks: OrchestrationQualityGateCheck[];
  release_readiness: OrchestrationReleaseReadiness;
  latest_verification_report?: OrchestrationReport | null;
  latest_review_report?: OrchestrationReport | null;
  latest_release_artifact?: OrchestrationArtifact | null;
  evidence_artifacts: OrchestrationArtifact[];
  policy_blockers: OrchestrationPolicyDecision[];
}

export interface OrchestrationArchitectureComponent {
  id: string;
  kind: string;
  label: string;
  status: 'active' | 'ready' | 'planned' | 'warning' | 'blocked' | string;
  isolation_level: string;
  summary: string;
  scope_kind?: string | null;
  scope_value?: string | null;
  evidence?: string | null;
}

export interface OrchestrationTenantArchitectureState {
  posture: 'single_user_ready' | 'hosted_ready' | 'enterprise_ready' | 'needs_design' | 'blocked' | string;
  summary: string;
  updated_at: string;
  tenant_id: string;
  user_id: string;
  project_scope: string;
  tenancy_model: string;
  deployment_model: string;
  isolation_boundaries: OrchestrationArchitectureComponent[];
  storage_partitions: OrchestrationArchitectureComponent[];
  worker_topology: OrchestrationArchitectureComponent[];
  event_streams: OrchestrationArchitectureComponent[];
  secret_boundaries: OrchestrationArchitectureComponent[];
  retention_policy: Record<string, any>;
  backup_policy: Record<string, any>;
  ha_readiness: {
    status: 'ready' | 'partial' | 'needs_design' | 'blocked' | string;
    summary: string;
    gaps: string[];
  };
  integration_accounts: Array<{
    id: string;
    provider: string;
    label: string;
    status: string;
    scope: string;
  }>;
  risks: string[];
  recommendations: string[];
}

export interface OrchestrationApiEndpoint {
  id: string;
  category: string;
  method: string;
  path: string;
  purpose: string;
  request_type?: string | null;
  response_type?: string | null;
  auth: string;
  streaming?: boolean;
  audit_surface?: string | null;
}

export interface OrchestrationApiManifest {
  version: string;
  generated_at: string;
  user_id?: string;
  base_path: string;
  auth: string;
  endpoints: OrchestrationApiEndpoint[];
  sdk_surfaces: Array<{
    id: string;
    label: string;
    summary: string;
    endpoints: string[];
  }>;
  event_stream: {
    protocol: string;
    endpoint: string;
    event_types: string[];
    replay: string;
  };
  governance_integrations: string[];
  schema_refs: Record<string, string>;
}

export interface OrchestrationRunDossier {
  exported_at: string;
  api_manifest: OrchestrationApiManifest;
  detail: OrchestrationRunDetail;
  audit_summary: Record<string, any>;
  integration_contract: Record<string, any>;
}

export interface OrchestrationRunDetail {
  run: OrchestrationRun;
  progress?: OrchestrationRunProgress | null;
  definition?: OrchestrationDefinition | null;
  campaign_state?: OrchestrationCampaignState | null;
  project_state?: OrchestrationProjectState | null;
  manager_loop?: OrchestrationManagerLoopState | null;
  observability?: OrchestrationObservabilityState | null;
  reliability?: OrchestrationReliabilityState | null;
  budget_state?: OrchestrationBudgetState | null;
  quality_gates?: OrchestrationQualityGateState | null;
  tenant_architecture?: OrchestrationTenantArchitectureState | null;
  process_streams?: OrchestrationProcessStreams | null;
  team_collaboration?: OrchestrationTeamCollaborationState | null;
  team_messages?: OrchestrationTeamMessage[];
  agents: OrchestrationAgent[];
  tasks: OrchestrationTask[];
  assignments: OrchestrationAssignment[];
  reports: OrchestrationReport[];
  decisions: OrchestrationDecision[];
  events: OrchestrationEvent[];
  artifacts: OrchestrationArtifact[];
  checkpoints: OrchestrationCheckpoint[];
  policy_decisions?: OrchestrationPolicyDecision[];
  audit_records?: OrchestrationAuditRecord[];
  ledger_entries?: OrchestrationLedgerEntry[];
  budget_entries?: OrchestrationBudgetEntry[];
  access_grants?: OrchestrationAccessGrant[];
}

export interface CreateOrchestrationRequest {
  name: string;
  goal: string;
  project_path: string;
  project_source?: OrchestrationProjectSource | any;
  target_kind?: OrchestrationTargetKind;
  runner_id?: string | null;
  model?: string;
  reasoning_effort?: string | null;
  permission_mode?: string;
  team_template?: string;
  team_config?: OrchestrationTeamRole[] | null;
  campaign_kind?: OrchestrationCampaignKind | string;
  operating_mode?: OrchestrationOperatingMode | string;
  governance_profile?: OrchestrationGovernanceProfile | string;
  campaign_phase?: OrchestrationCampaignPhase | string;
  scope?: any;
  success_criteria?: any;
  data_policy?: OrchestrationDataPolicy | any;
  connector_registry?: OrchestrationConnector[] | any;
  schedule_kind?: OrchestrationScheduleKind;
  interval_secs?: number | null;
  enabled?: boolean;
  max_runtime_secs?: number | null;
  max_tokens?: number | null;
  max_cost_usd?: number | null;
  checkpoint_policy?: 'automatic' | 'manual' | 'off' | string;
  completion_condition?: string | null;
}

export type UpdateOrchestrationRequest = Partial<CreateOrchestrationRequest>;

// MCP server config
export interface McpServerConfig {
  id?: string;
  projectPath?: string | null;
  name: string;
  command: string;
  args?: string[];
  enabled?: boolean;
  isBuiltin?: boolean;
  description?: string;
}

export type CapabilityTransport = 'stdio' | 'streamable_http' | 'sse' | 'native';
export type CapabilityStatus =
  | 'available'
  | 'configured'
  | 'ready'
  | 'needs_auth'
  | 'failed'
  | 'unavailable'
  | 'disabled';

export interface CapabilityCatalogEntry {
  id: string;
  slug: string;
  title: string;
  description: string;
  category: string;
  source: 'builtin' | 'admin' | 'registry' | 'legacy' | string;
  publisher: string;
  version: string;
  transport: CapabilityTransport;
  command?: string | null;
  args: string[];
  remoteUrl?: string | null;
  authKind: 'none' | 'oauth2' | 'token' | string;
  permissions: string[];
  toolsets: string[];
  adminApproved: boolean;
  builtin: boolean;
  connected: boolean;
  connectionId?: string | null;
  connectionName?: string | null;
  enabled: boolean;
  status: CapabilityStatus;
  lastError?: string | null;
  projectPath?: string | null;
  latencyMs?: number | null;
  toolCount: number;
  consecutiveFailures: number;
  restartCount: number;
  circuitState: 'closed' | 'open' | 'half_open' | string;
  lastSuccessAt?: string | null;
  packId?: string | null;
  packVersion?: string | null;
  packPreviousVersion?: string | null;
  packState?: 'installed' | 'uninstalled' | string | null;
  packSigned: boolean;
  scheduleEligible: boolean;
  nativeAdapter?: string | null;
}

export interface CapabilityTool {
  capabilityId: string;
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
  outputSchema?: Record<string, unknown> | null;
  annotations: Record<string, unknown>;
  enabled: boolean;
  discoveredAt: string;
}

export interface MarketplaceCapability {
  id: string;
  slug: string;
  title: string;
  description: string;
  category: string;
  publisher: string;
  official: boolean;
  homepageUrl?: string | null;
  transport: CapabilityTransport;
  remoteUrl?: string | null;
  authKind: string;
  permissions: string[];
  status: string;
  metadata?: {
    packId?: string;
    packVersion?: string;
    packState?: string;
    signed?: boolean;
    scheduleEligible?: boolean;
    [key: string]: unknown;
  };
}

export interface CapabilityPackInstallation {
  packId: string;
  capabilityId?: string | null;
  activeVersion?: string | null;
  previousVersion?: string | null;
  state: 'installed' | 'uninstalled' | string;
  scheduleEligible: boolean;
  schedulesEnabled: boolean;
  installedBy: string;
  installedAt: string;
  updatedAt: string;
}

export interface GovernedSkill {
  id: string;
  slug: string;
  version: string;
  title: string;
  description: string;
  category: string;
  instructions: string;
  triggers: string[];
  contentHash: string;
  requiredCapabilities: string[];
  allowedOperations: string[];
  providerCompatibility: Array<'codex' | 'claude' | string>;
  scope: 'global' | 'personal' | 'project' | string;
  ownerUserId?: string | null;
  projectPath?: string | null;
  provenance: Record<string, unknown>;
  reviewState: 'proposed' | 'approved' | 'rejected' | string;
  signed: boolean;
  quarantined: boolean;
  quarantineReason?: string | null;
  builtin: boolean;
  installed: boolean;
  installState: 'available' | 'active' | 'disabled' | string;
  activeVersion?: string | null;
  previousVersion?: string | null;
  providerProjectionEnabled: boolean;
  installationScope?: 'personal' | 'project' | null;
  snapshotRevision?: string | null;
  active: boolean;
}

export interface GovernedSkillVersionInput {
  version: string;
  title: string;
  description?: string;
  category: string;
  instructions: string;
  triggers?: string[];
  requiredCapabilities?: string[];
  allowedOperations?: string[];
  providerCompatibility?: Array<'codex' | 'claude'>;
  scope?: 'global' | 'personal' | 'project';
  projectPath?: string | null;
  provenance?: Record<string, unknown>;
}

export interface SkillProjectionSnapshot {
  provider: 'codex' | 'claude' | string;
  root: string;
  revision: string;
  skills: string[];
}

export interface WorkItem {
  id: string;
  userId: string;
  projectPath: string;
  parentId?: string | null;
  kind: string;
  title: string;
  description: string;
  status: string;
  priority: string;
  assigneeKind?: string | null;
  assigneeId?: string | null;
  milestone?: string | null;
  releaseStatus?: string | null;
  planId?: string | null;
  orchestrationRunId?: string | null;
  labels: string[];
  position: number;
  createdAt: string;
  updatedAt: string;
}

export interface CapabilityCatalogResponse {
  capabilities: CapabilityCatalogEntry[];
  adminManaged: boolean;
  autonomy: 'maximum' | 'risk_based' | 'confirm_writes' | string;
}

// Share
export interface ShareToken {
  token: string;
  sessionId: string;
  createdAt: string;
}

// Prompt
export interface SavedPrompt {
  id: string;
  title: string;
  content: string;
  createdAt: string;
  useCount: number;
}

// Project
export interface Project {
  path: string;
  name: string;
}

// Git
export interface GitStatus {
  repositoryUrl?: string;
  revision?: string;
  branch: string;
  files: GitFileStatus[];
  added: number;
  removed: number;
  uncommittedCount: number;
  isRepo: boolean;
}

export type GitChangeStatus = 'A' | 'M' | 'D' | 'R' | 'C' | 'T' | 'U' | '?' | '!';
export type GitDiffMode = 'worktree' | 'staged' | 'head';

export interface GitFileStatus {
  originalPath?: string;
  status: GitChangeStatus;
  path: string;
  added?: number;
  removed?: number;
  staged?: boolean;
  unstaged?: boolean;
  untracked?: boolean;
  indexStatus?: GitChangeStatus | ' ';
  worktreeStatus?: GitChangeStatus | ' ';
  stagedAdded?: number;
  stagedRemoved?: number;
  unstagedAdded?: number;
  unstagedRemoved?: number;
}

export type WorkbenchResourceKind =
  | 'chat'
  | 'file'
  | 'diff'
  | 'image'
  | 'terminal'
  | 'search'
  | 'problems'
  | 'pull_request'
  | 'action_run';

export interface WorkbenchResource {
  id: string;
  kind: WorkbenchResourceKind;
  projectPath: string;
  title: string;
  subtitle?: string;
  path?: string;
  artifactId?: string;
  commandSessionId?: string;
  repositoryRef?: string;
  dirty?: boolean;
  pinned?: boolean;
  preview?: boolean;
  metadata?: Record<string, unknown>;
}

export interface FileRevision {
  path: string;
  content: string;
  revision: string;
  mediaType: string;
  size: number;
  modifiedAt?: string;
  binary?: boolean;
}

export interface FileWriteRequest {
  path: string;
  content: string;
  expectedRevision?: string;
}

export interface FileWriteConflict {
  code: 'revision_conflict';
  path: string;
  expectedRevision?: string;
  current: FileRevision;
}

export type GitOperationKind =
  | 'stage'
  | 'unstage'
  | 'discard'
  | 'commit'
  | 'amend'
  | 'fetch'
  | 'pull'
  | 'push'
  | 'sync'
  | 'branch_create'
  | 'branch_checkout'
  | 'branch_delete'
  | 'tag_create'
  | 'stash_push'
  | 'stash_apply'
  | 'stash_pop'
  | 'merge'
  | 'rebase'
  | 'cherry_pick'
  | 'revert';

export interface GitOperationRequest {
  expectedRevision?: string;
  path: string;
  operation: GitOperationKind;
  files?: string[];
  message?: string;
  target?: string;
  remote?: string;
  forceWithLease?: boolean;
}

export interface GitOperationResult {
  ok: boolean;
  operation: GitOperationKind;
  stdout: string;
  stderr: string;
  exitCode: number;
  status?: GitStatus;
}

export interface GitCommitSummary {
  id: string;
  shortId: string;
  parents: string[];
  authorName: string;
  authorEmail: string;
  authoredAt: string;
  subject: string;
  refs: string[];
}

export interface GitBranchSummary {
  name: string;
  current: boolean;
  remote?: string;
  upstream?: string;
  ahead?: number;
  behind?: number;
  commit?: string;
}

export interface GitDiffLine {
  kind: 'addition' | 'deletion' | 'context' | 'hunk' | 'metadata' | 'no_newline';
  content: string;
  oldLine?: number;
  newLine?: number;
}

export interface GitDiffFile {
  oldPath?: string;
  newPath?: string;
  status: GitChangeStatus;
  binary: boolean;
  additions: number;
  deletions: number;
  lines: GitDiffLine[];
}

export type DiagnosticSeverity = 'error' | 'warning' | 'information' | 'hint';

export interface WorkbenchDiagnostic {
  id: string;
  path: string;
  source: string;
  message: string;
  severity: DiagnosticSeverity;
  startLine: number;
  startColumn: number;
  endLine?: number;
  endColumn?: number;
  code?: string;
}

export type ArtifactKind = 'generated_image' | 'image' | 'screenshot' | 'archive' | 'log' | 'file';

export interface ArtifactRef {
  id: string;
  kind: ArtifactKind;
  name: string;
  mediaType: string;
  byteSize: number;
  sha256: string;
  createdAt: string;
  width?: number;
  height?: number;
  sessionId?: string;
  toolCallId?: string;
  thumbnailUrl?: string;
  contentUrl: string;
  metadata?: Record<string, unknown>;
}

// File tree
export type FileReferenceKind = 'file' | 'directory';

export interface FileReference {
  projectPath: string;
  path: string;
  relativePath: string;
  name: string;
  kind: FileReferenceKind;
  source?: 'tree' | 'editor' | 'tool' | 'diff' | 'chat';
  revision?: string;
}

export interface FileNode {
  name: string;
  path: string;
  type: FileReferenceKind;
  children?: FileNode[];
  modified?: boolean; // modified by Claude in current session
  dirty?: boolean;
  generated?: boolean;
  gitStatus?: GitFileStatus['status'];
  size?: number;
  modifiedAt?: string;
}

// Usage stats
export interface UsageStats {
  today: PeriodUsage;
  week: PeriodUsage;
  month: PeriodUsage;
  byProject: ProjectUsage[];
}

export interface PeriodUsage {
  inputTokens: number;
  outputTokens: number;
}

export interface ProjectUsage {
  projectPath: string;
  inputTokens: number;
  outputTokens: number;
}

// Todo
export interface TodoItem {
  id: string;
  text: string;
  done: boolean;
  position: number;
  createdAt: string;
}

// Plan
export type PlanExecutionStatus = 'ready' | 'running' | 'paused' | 'blocked' | 'completed';
export type PlanApprovalStatus = 'pending' | 'approved' | 'not_required';

export interface PlanItem {
  id: string;
  text: string;
  status: 'todo' | 'in_progress' | 'done' | 'blocked';
  notes?: string;
  section?: string;
  reasoningFloor?: 'low' | 'medium' | 'high' | 'xhigh' | 'max' | 'ultra';
  position: number;
}

export interface PlanArtifact {
  id: string;
  planId: string;
  kind: 'plan' | string;
  schemaVersion: number;
  sessionId?: string;
  projectPath?: string;
  metadata: Record<string, unknown>;
  createdAt: string;
  updatedAt: string;
}

export interface PlanMeta {
  id: string;
  title: string;
  description?: string;
  sessionId?: string;
  projectPath?: string;
  itemCount: number;
  doneCount: number;
  versionCount?: number;
  createdAt: string;
  updatedAt: string;
  shareToken?: string;
  finished?: boolean;
  archived?: boolean;
  executionStatus?: PlanExecutionStatus;
  approvalStatus?: PlanApprovalStatus;
  activeItemId?: string;
  approvalRequestedAt?: string;
  approvedAt?: string;
  executionStartedAt?: string;
  executionUpdatedAt?: string;
  artifact?: PlanArtifact;
}

export interface Plan extends PlanMeta {
  items: PlanItem[];
  versions?: PlanVersion[];
}

export interface PlanVersion {
  id: string;
  items: PlanItem[];
  timestamp: string;
  schemaVersion?: number;
  title?: string;
  description?: string;
  artifact?: PlanArtifact;
}

export interface GitAccount {
  id: string;
  type: 'github' | 'gitea' | 'flamel';
  label: string;
  baseUrl: string;
  token: string;
  createdAt: string;
  username?: string;
  avatarUrl?: string;
}

export interface GitRepo {
  owner: string;
  name: string;
  fullName: string;
  defaultBranch: string;
  htmlUrl: string;
}

export interface GitIssue {
  number: number;
  title: string;
  body: string;
  state: 'open' | 'closed';
  labels: string[];
  author: string;
  htmlUrl: string;
  createdAt: string;
}

export interface GitPullRequest {
  number: number;
  title: string;
  body: string;
  state: 'open' | 'closed' | 'merged';
  headBranch: string;
  baseBranch: string;
  author: string;
  htmlUrl: string;
  draft: boolean;
  createdAt: string;
  diff?: string;
}

export interface AgentConfig {
  id: string;
  gitAccountId: string;
  repoOwner: string;
  repoName: string;
  triggerEvent: string;
  triggerLabel?: string;
  projectPath: string;
  model: string;
  permissionMode: string;
  autoPr: boolean;
  autoClose: boolean;
  commentOnStart: boolean;
  requireApproval: boolean;
  enabled: boolean;
  createdAt: string;
}

export interface AgentRun {
  id: string;
  agentConfigId: string;
  sessionId?: string;
  status: 'pending' | 'running' | 'awaiting_approval' | 'done' | 'failed';
  triggerPayload: any;
  prUrl?: string;
  error?: string;
  createdAt: string;
  updatedAt: string;
}
