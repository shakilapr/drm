// Display-only parser for the engine's canonical model render. The engine
// produces the canonical text; the renderer only presents it and never
// validates or mutates (I-ST1, REQ-STUDIO-003). This module holds zero DRM
// rules — it is a pure projection of engine output for the read-only
// inspectors (REQ-STUDIO-014).

/** One declared topic, as rendered by the engine. */
export interface TopicView {
  readonly path: string;
  readonly schemaHash: string;
  readonly schema: string;
}

/** One package with its topics. */
export interface PackageView {
  readonly name: string;
  readonly topics: TopicView[];
}

/** One node with its publish/consume interface. */
export interface NodeView {
  readonly name: string;
  readonly publishes: string[];
  readonly consumes: string[];
}

/** One compute unit. */
export interface ComputeView {
  readonly name: string;
}

/** One link between compute units. */
export interface LinkView {
  readonly id: string;
  readonly from: string;
  readonly to: string;
}

/** One node placement onto a compute unit. */
export interface PlacementView {
  readonly node: string;
  readonly compute: string;
}

/** Structured projection of a canonical model render. */
export interface ModelView {
  readonly packages: PackageView[];
  readonly nodes: NodeView[];
  readonly computes: ComputeView[];
  readonly links: LinkView[];
  readonly placements: PlacementView[];
}

/** Splits a whitespace-delimited token list. */
function tokens(rest: string): string[] {
  return rest
    .trim()
    .split(/\s+/)
    .filter((token) => token.length > 0);
}

/** Parses the engine's canonical model render into a `ModelView`. */
export function parseModel(render: string): ModelView {
  const packages: PackageView[] = [];
  const nodes: NodeView[] = [];
  const computes: ComputeView[] = [];
  const links: LinkView[] = [];
  const placements: PlacementView[] = [];

  let currentPackage: PackageView | null = null;
  let currentTopic: string | null = null;
  let topicHash = "";
  let topicSchema: string[] | null = null;
  let topicPhase: "hash" | "schema" | null = null;
  let currentNode: NodeView | null = null;

  for (const line of render.split("\n")) {
    if (topicPhase === "hash") {
      topicHash = line.trim();
      topicSchema = [];
      topicPhase = "schema";
      continue;
    }
    if (topicPhase === "schema" && topicSchema !== null) {
      if (line === "end") {
        if (currentTopic !== null && currentPackage !== null) {
          currentPackage.topics.push({
            path: currentTopic,
            schemaHash: topicHash,
            schema: topicSchema.join("\n"),
          });
        }
        currentTopic = null;
        topicSchema = null;
        topicPhase = null;
        continue;
      }
      topicSchema.push(line);
      continue;
    }
    if (line.startsWith("package ")) {
      currentPackage = { name: line.slice("package ".length).trim(), topics: [] };
      packages.push(currentPackage);
    } else if (line.startsWith("topic ")) {
      currentTopic = line.slice("topic ".length).trim();
      topicHash = "";
      topicSchema = [];
      topicPhase = "hash";
    } else if (line.startsWith("node ")) {
      currentNode = {
        name: line.slice("node ".length).trim(),
        publishes: [],
        consumes: [],
      };
      nodes.push(currentNode);
    } else if (line.startsWith("publishes") && currentNode !== null) {
      currentNode.publishes = tokens(line.slice("publishes".length));
    } else if (line.startsWith("consumes") && currentNode !== null) {
      currentNode.consumes = tokens(line.slice("consumes".length));
    } else if (line === "end") {
      currentNode = null;
    } else if (line.startsWith("compute ")) {
      computes.push({ name: line.slice("compute ".length).trim() });
    } else if (line.startsWith("link ")) {
      const [id = "", from = "", to = ""] = tokens(line.slice("link ".length));
      if (id !== "" && from !== "" && to !== "") {
        links.push({ id, from, to });
      }
    } else if (line.startsWith("place ")) {
      const [node = "", compute = ""] = tokens(line.slice("place ".length));
      if (node !== "" && compute !== "") {
        placements.push({ node, compute });
      }
    }
  }

  return { packages, nodes, computes, links, placements };
}

/** All topics across every package, flattened for list/table views. */
export function allTopics(model: ModelView): Array<TopicView & { package: string }> {
  return model.packages.flatMap((pkg) =>
    pkg.topics.map((topic) => ({ ...topic, package: pkg.name })),
  );
}
