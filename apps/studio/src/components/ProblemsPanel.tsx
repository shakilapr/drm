// Problems panel: first-class verify diagnostics with severity classes that
// mirror the verify model — ERROR (proven), WARNING (feasibility estimate),
// RUNTIME (observed), INFO — never collapsed into a generic red (ui-arch
// §Problems Panel, REQ-STUDIO-014). The panel renders engine findings only; it
// computes no diagnostics of its own (I-ST1).
import type { Diagnostic, Severity } from "@drm/studio-protocol";

const SEVERITY_ORDER: readonly Severity[] = ["error", "warning", "runtime", "info"];

const SEVERITY_LABEL: Record<Severity, string> = {
  error: "ERROR",
  warning: "WARN",
  runtime: "RUNTIME",
  info: "INFO",
};

const SEVERITY_CLASS: Record<Severity, string> = {
  error: "severity-error",
  warning: "severity-warning",
  runtime: "severity-runtime",
  info: "severity-info",
};

/** Props for {@link ProblemsPanel}. */
export interface ProblemsPanelProps {
  readonly problems: readonly Diagnostic[];
}

/** Renders verify findings grouped by severity class. */
export function ProblemsPanel({ problems }: ProblemsPanelProps): React.JSX.Element {
  const groups = SEVERITY_ORDER.map((severity) => ({
    severity,
    items: problems.filter((problem) => problem.severity === severity),
  })).filter((group) => group.items.length > 0);

  return (
    <section aria-label="Problems" data-testid="problems-panel">
      {groups.length === 0 ? (
        <p className="problems-empty">No problems</p>
      ) : (
        <ul className="problems-list">
          {groups.flatMap((group) =>
            group.items.map((problem, index) => (
              <li
                key={`${group.severity}-${String(index)}`}
                className={`problem ${SEVERITY_CLASS[group.severity]}`}
              >
                <span className="problem-severity">{SEVERITY_LABEL[group.severity]}</span>
                <span className="problem-message">{problem.message}</span>
              </li>
            )),
          )}
        </ul>
      )}
    </section>
  );
}
