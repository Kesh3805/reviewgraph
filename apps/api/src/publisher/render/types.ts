/**
 * View model the renderers consume. API-010 (finding detail) maps verified findings onto it, so
 * the renderers stay pure and independent of persistence and of provider payloads.
 */
export type Severity = 'info' | 'low' | 'medium' | 'high' | 'critical';

/** Wire form of the contract `DiffSide`: `head` is the new file (RIGHT), `base` the old (LEFT). */
export type FindingSide = 'base' | 'head';

export interface FindingLocation {
  path: string;
  /** 1-based inclusive line range on `side`. */
  startLine: number;
  endLine: number;
  side: FindingSide;
}

export interface EvidenceItem {
  claim: string;
  path?: string;
  line?: number;
}

export interface RenderableFinding {
  id: string;
  /** Human-facing short id, for example `RG-142-003`. */
  shortId: string;
  fingerprint: string;
  reviewRunId: string;
  severity: Severity;
  title: string;
  /** PRD section 59 Q1. */
  whatChanged: string;
  /** Q2. */
  whyRisky: string;
  /** Q4. */
  behaviorResult: string;
  /** Q5. */
  correctiveDirection: string;
  /** Q3: the call/impact path as display names, nearest the change first. */
  evidencePath?: string[];
  /** Cited instead of a path when no path exists. */
  evidenceItems?: EvidenceItem[];
  reviewer: string;
  confidence: number;
  location?: FindingLocation;
  /** True when no current caller reaches the defect. */
  latent?: boolean;
}

export type RelocationReason = 'outside_diff' | 'inline_cap' | 'render_error';

export interface InlineAnchor {
  kind: 'inline';
  path: string;
  line: number;
  side: 'LEFT' | 'RIGHT';
  startLine?: number;
  startSide?: 'LEFT' | 'RIGHT';
}

export interface SummaryAnchor {
  kind: 'summary';
  reason: Extract<RelocationReason, 'outside_diff'>;
}

export type Anchor = InlineAnchor | SummaryAnchor;
