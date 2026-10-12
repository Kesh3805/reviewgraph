//! File templates. Every emitted declaration is counted as it is written, so the manifest is exact
//! by construction rather than estimated.

use std::collections::BTreeSet;

use crate::manifest::Counts;
use crate::profile::ProfileSpec;
use crate::rng::{mix, Rng};

const NOUNS: &[&str] = &[
    "account",
    "invoice",
    "ledger",
    "payment",
    "refund",
    "customer",
    "supplier",
    "order",
    "shipment",
    "product",
    "catalog",
    "pricing",
    "discount",
    "coupon",
    "cart",
    "checkout",
    "warehouse",
    "inventory",
    "carrier",
    "route",
    "driver",
    "vehicle",
    "ticket",
    "booking",
    "schedule",
    "calendar",
    "reminder",
    "message",
    "channel",
    "thread",
    "comment",
    "review",
    "rating",
    "profile",
    "session",
    "device",
    "token",
    "policy",
    "permission",
    "role",
    "tenant",
    "billing",
    "subscription",
    "plan",
    "quota",
    "usage",
    "report",
    "metric",
    "audit",
    "export",
    "import",
    "document",
    "template",
    "workflow",
    "approval",
    "task",
    "project",
    "milestone",
    "budget",
    "forecast",
    "contract",
    "vendor",
    "asset",
    "location",
];

const VERBS: &[&str] = &[
    "reconcile",
    "validate",
    "recalculate",
    "archive",
    "notify",
    "enrich",
    "normalize",
    "sync",
    "audit",
    "dispatch",
    "schedule",
    "estimate",
    "migrate",
    "aggregate",
    "publish",
    "classify",
];

const PROP_NAMES: &[&str] = &[
    "name",
    "title",
    "status",
    "amount",
    "currency",
    "owner",
    "region",
    "notes",
    "priority",
    "score",
    "code",
    "label",
    "kind",
    "reference",
    "email",
    "phone",
];

const PROP_TYPES: &[&str] = &["string", "number", "boolean", "Date"];

const REPO_METHODS: &[&str] = &[
    "findAll",
    "findOne",
    "create",
    "update",
    "remove",
    "count",
    "exists",
    "findByIds",
    "softDelete",
    "restore",
];

const CORE_METHODS: &[&str] = &["findAll", "findOne", "create", "update", "remove"];

const ROUTES: &[(&str, &str, &str, &str)] = &[
    ("Get", "", "findAll", "findAll"),
    ("Get", ":id", "findOne", "findOne"),
    ("Post", "", "create", "create"),
    ("Put", ":id", "update", "update"),
    ("Delete", ":id", "remove", "remove"),
    ("Get", "count", "count", "findAll"),
    ("Patch", ":id/restore", "restore", "findOne"),
    ("Get", "export", "exportAll", "findAll"),
];

/// One generated file.
#[derive(Debug, Clone)]
pub struct FileOut {
    pub path: String,
    pub content: String,
}

/// Everything one module (or the shared folder) contributes.
#[derive(Debug, Default)]
pub struct Out {
    pub files: Vec<FileOut>,
    pub counts: Counts,
}

impl Out {
    fn file(&mut self, path: String, content: String) {
        self.counts.files += 1;
        self.counts.bytes += content.len() as u64;
        self.counts.imports += content
            .lines()
            .filter(|line| line.starts_with("import "))
            .count() as u64;
        self.files.push(FileOut { path, content });
    }
}

/// Global generation plan: profile, seed, hub list and the Zipf table.
#[derive(Debug, Clone)]
pub struct Plan {
    pub spec: ProfileSpec,
    pub seed: u64,
    hubs: Vec<(u32, u32)>,
    cdf: Vec<u64>,
}

impl Plan {
    pub fn new(spec: ProfileSpec, seed: u64) -> Self {
        let total = spec.total_features().max(1);
        let hub_count = total.min(20);
        let features = u64::from(spec.features_per_module.max(1));
        let offset = seed % total;
        let mut hubs = Vec::new();
        let mut cdf = Vec::new();
        let mut acc = 0u64;
        for rank in 0..hub_count {
            let global = (rank * 7919 + offset) % total;
            hubs.push(((global / features) as u32, (global % features) as u32));
            acc += 1_000_000 / (rank + 1);
            cdf.push(acc);
        }
        Self {
            spec,
            seed,
            hubs,
            cdf,
        }
    }

    pub fn total_features(&self) -> u64 {
        self.spec.total_features()
    }

    pub fn hubs(&self) -> &[(u32, u32)] {
        &self.hubs
    }

    pub fn util_count(&self) -> u32 {
        (self.total_features().div_ceil(100)).max(2) as u32
    }

    fn pick_hub(&self, rng: &mut Rng) -> usize {
        let total = self.cdf.last().copied().unwrap_or(1);
        let x = rng.below(total);
        self.cdf
            .partition_point(|&c| c <= x)
            .min(self.hubs.len().saturating_sub(1))
    }

    fn scale(&self, n: u64) -> u64 {
        (n * u64::from(self.spec.density_pct) + 50) / 100
    }

    /// Every feature that gets a `generated/` file (about 3%).
    pub fn has_generated(&self, mi: u32, fi: u32) -> bool {
        mix(self.seed, u64::from(mi), u64::from(fi) + 1000) % 33 == 0
    }

    pub fn has_processor(fi: u32) -> bool {
        fi % 5 == 0
    }
}

/// Names and paths of one feature.
#[derive(Debug, Clone)]
pub struct Ident {
    pub pascal: String,
    pub kebab: String,
    pub dir: String,
    pub mod_dir: String,
}

impl Ident {
    pub fn service_path(&self) -> String {
        format!("{}/{}.service.ts", self.dir, self.kebab)
    }
}

fn capitalise(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

pub fn ident(seed: u64, mi: u32, fi: u32) -> Ident {
    let noun = NOUNS[(mix(seed, u64::from(mi), u64::from(fi)) % NOUNS.len() as u64) as usize];
    let mod_noun =
        NOUNS[(mix(seed, u64::from(mi), u64::from(u32::MAX)) % NOUNS.len() as u64) as usize];
    let kebab = format!("{noun}-m{mi}f{fi}");
    let mod_dir = format!("mod{mi}-{mod_noun}");
    Ident {
        pascal: format!("{}M{mi}F{fi}", capitalise(noun)),
        dir: format!("src/{mod_dir}/{kebab}"),
        kebab,
        mod_dir,
    }
}

fn prop_lines(count: u64, decorator: &str) -> String {
    let mut s = String::new();
    for i in 0..count {
        let name = PROP_NAMES[(i as usize) % PROP_NAMES.len()];
        let ty = PROP_TYPES[(i as usize) % PROP_TYPES.len()];
        s.push_str(&format!("  {decorator}{name}{i}: {ty};\n"));
    }
    s
}

#[derive(Default)]
struct SvcState {
    uses_other: bool,
    utils: BTreeSet<(u32, u32)>,
    hub: Option<usize>,
}

/// One planted statement inside a service method.
fn statement(
    plan: &Plan,
    mi: u32,
    fi: u32,
    rng: &mut Rng,
    st: &mut SvcState,
    has_ref: bool,
    counts: &mut Counts,
) -> String {
    if has_ref && rng.below(1000) < 20 {
        counts.calls_ambiguous += 1;
        return "    ref.findOne(id);\n".to_owned();
    }
    let features = plan.spec.features_per_module;
    let roll = rng.below(100);
    if (80..95).contains(&roll) {
        let file = rng.below(u64::from(plan.util_count())) as u32;
        let func = rng.below(4) as u32;
        st.utils.insert((file, func));
        counts.calls_shared += 1;
        return format!("    util{file}F{func}(id);\n");
    }
    if roll >= 95 {
        let rank = *st.hub.get_or_insert_with(|| plan.pick_hub(rng));
        if plan.hubs.get(rank).copied() != Some((mi, fi)) {
            let method = CORE_METHODS[rng.below(2) as usize];
            counts.calls_cross_module += 1;
            *counts.hub_fanin.entry(rank as u32).or_insert(0) += 1;
            return format!("    await this.hub.{method}(id);\n");
        }
    }
    counts.calls_di += 1;
    if features > 1 && rng.below(100) < 40 {
        st.uses_other = true;
        let method = CORE_METHODS[rng.below(CORE_METHODS.len() as u64) as usize];
        return format!("    await this.other.{method}(id);\n");
    }
    let method = REPO_METHODS[rng.below(REPO_METHODS.len() as u64) as usize];
    format!("    await this.repo.{method}(id);\n")
}

fn service(plan: &Plan, mi: u32, fi: u32, rng: &mut Rng, out: &mut Out) -> String {
    let id = ident(plan.seed, mi, fi);
    let p = &id.pascal;
    let k = &id.kebab;
    let extras = plan.scale(rng.range(4, 12));
    let mut st = SvcState::default();
    let mut methods = String::new();
    let mut names: Vec<(String, String, bool)> = CORE_METHODS
        .iter()
        .map(|m| {
            let sig = match *m {
                "create" => format!("dto: Create{p}Dto"),
                "update" => format!("id: string, dto: Update{p}Dto"),
                _ => "id: string".to_owned(),
            };
            ((*m).to_owned(), sig, false)
        })
        .collect();
    for n in 0..extras {
        let verb =
            VERBS[((n + mix(plan.seed, u64::from(mi), u64::from(fi))) as usize) % VERBS.len()];
        names.push((
            format!("{verb}{n}"),
            "id: string, ref: any".to_owned(),
            true,
        ));
    }
    for (name, sig, has_ref) in &names {
        methods.push_str(&format!("  async {name}({sig}): Promise<unknown> {{\n"));
        if name.as_str() == "create" {
            methods.push_str("    const id = String(dto);\n");
        }
        methods.push_str("    const limit = 100;\n");
        let stmts = rng.range(3, 6);
        for _ in 0..stmts {
            methods.push_str(&statement(
                plan,
                mi,
                fi,
                rng,
                &mut st,
                *has_ref,
                &mut out.counts,
            ));
        }
        methods.push_str("    return limit;\n  }\n");
    }
    out.counts.sym("method", names.len() as u64);

    let mut s = String::new();
    s.push_str("import { Injectable } from '@nestjs/common';\n");
    s.push_str(&format!(
        "import {{ {p}Repository }} from './{k}.repository';\n"
    ));
    s.push_str(&format!(
        "import {{ Create{p}Dto }} from './dto/create-{k}.dto';\n"
    ));
    s.push_str(&format!(
        "import {{ Update{p}Dto }} from './dto/update-{k}.dto';\n"
    ));
    let mut params = format!("private readonly repo: {p}Repository");
    let mut props = 1u64;
    if st.uses_other {
        let other = ident(plan.seed, mi, (fi + 1) % plan.spec.features_per_module);
        s.push_str(&format!(
            "import {{ {op}Service }} from '../{ok}/{ok}.service';\n",
            op = other.pascal,
            ok = other.kebab
        ));
        params.push_str(&format!(
            ", private readonly other: {}Service",
            other.pascal
        ));
        props += 1;
    }
    if let Some(rank) = st.hub {
        if let Some(&(hm, hf)) = plan.hubs.get(rank) {
            let hub = ident(plan.seed, hm, hf);
            if st.uses_hub_calls(&methods) {
                s.push_str(&format!(
                    "import {{ {hp}Service }} from '../../{hd}/{hk}/{hk}.service';\n",
                    hp = hub.pascal,
                    hd = hub.mod_dir,
                    hk = hub.kebab
                ));
                params.push_str(&format!(", private readonly hub: {}Service", hub.pascal));
                props += 1;
            }
        }
    }
    let mut by_file: std::collections::BTreeMap<u32, Vec<u32>> = std::collections::BTreeMap::new();
    for (file, func) in &st.utils {
        by_file.entry(*file).or_default().push(*func);
    }
    for (file, funcs) in &by_file {
        let list: Vec<String> = funcs.iter().map(|f| format!("util{file}F{f}")).collect();
        s.push_str(&format!(
            "import {{ {} }} from '../../shared/util-{file}';\n",
            list.join(", ")
        ));
    }
    s.push_str(&format!(
        "\n@Injectable()\nexport class {p}Service {{\n  constructor({params}) {{}}\n\n{methods}}}\n"
    ));
    out.counts.sym("class", 1);
    out.counts.sym("constructor", 1);
    out.counts.sym("property", props);
    s
}

impl SvcState {
    fn uses_hub_calls(&self, methods: &str) -> bool {
        methods.contains("this.hub.")
    }
}

fn controller(id: &Ident, rng: &mut Rng, out: &mut Out) -> String {
    let p = &id.pascal;
    let k = &id.kebab;
    let routes = rng.range(4, 8) as usize;
    let mut verbs: BTreeSet<&str> = BTreeSet::new();
    for (verb, _, _, _) in ROUTES.iter().take(routes) {
        verbs.insert(verb);
    }
    let mut imports: Vec<&str> = vec!["Controller", "UseGuards"];
    imports.extend(verbs.iter().copied());
    let mut s = String::new();
    s.push_str(&format!(
        "import {{ {} }} from '@nestjs/common';\n",
        imports.join(", ")
    ));
    s.push_str("import { AuthGuard } from '../../shared/auth.guard';\n");
    s.push_str(&format!(
        "import {{ {p}Service }} from './{k}.service';\n\n"
    ));
    s.push_str(&format!(
        "@Controller('{k}')\nexport class {p}Controller {{\n  constructor(private readonly service: {p}Service) {{}}\n"
    ));
    for (verb, path, name, target) in ROUTES.iter().take(routes) {
        let arg = if *target == "create" {
            "{}"
        } else if *target == "update" {
            "'x', {}"
        } else {
            "'x'"
        };
        s.push_str(&format!(
            "\n  @{verb}('{path}')\n  @UseGuards(AuthGuard)\n  {name}() {{\n    return this.service.{target}({arg});\n  }}\n"
        ));
    }
    s.push_str("}\n");
    out.counts.routes += routes as u64;
    out.counts.sym("class", 1);
    out.counts.sym("constructor", 1);
    out.counts.sym("property", 1);
    out.counts.sym("method", routes as u64);
    s
}

fn repository(id: &Ident, out: &mut Out) -> String {
    let p = &id.pascal;
    let k = &id.kebab;
    let mut s = String::new();
    s.push_str("import { Injectable } from '@nestjs/common';\n");
    s.push_str(&format!("import {{ {p}Entity }} from './{k}.entity';\n\n"));
    s.push_str(&format!(
        "@Injectable()\nexport class {p}Repository {{\n  private readonly rows = new Map<string, {p}Entity>();\n"
    ));
    for method in REPO_METHODS {
        s.push_str(&format!(
            "\n  async {method}(id: string): Promise<unknown> {{\n    return this.rows.get(id);\n  }}\n"
        ));
    }
    s.push_str("}\n");
    out.counts.sym("class", 1);
    out.counts.sym("property", 1);
    out.counts.sym("method", REPO_METHODS.len() as u64);
    s
}

fn entity(plan: &Plan, id: &Ident, rng: &mut Rng, out: &mut Out) -> String {
    let p = &id.pascal;
    let props = plan.scale(rng.range(10, 14));
    let mut s = String::new();
    s.push_str("import { Entity, Column, PrimaryGeneratedColumn } from 'typeorm';\n\n");
    s.push_str(&format!(
        "@Entity('{}')\nexport class {p}Entity {{\n  @PrimaryGeneratedColumn('uuid')\n  id: string;\n",
        id.kebab.replace('-', "_")
    ));
    s.push_str(&prop_lines(props, "@Column() "));
    s.push_str("}\n");
    out.counts.entities += 1;
    out.counts.sym("class", 1);
    out.counts.sym("property", props + 1);
    s
}

fn dto(plan: &Plan, id: &Ident, create: bool, rng: &mut Rng, out: &mut Out) -> String {
    let p = &id.pascal;
    let (name, props) = if create {
        (format!("Create{p}Dto"), plan.scale(rng.range(6, 9)))
    } else {
        (format!("Update{p}Dto"), plan.scale(rng.range(3, 5)))
    };
    let mut s = String::new();
    s.push_str("import { IsOptional } from 'class-validator';\n\n");
    s.push_str(&format!("export class {name} {{\n"));
    s.push_str(&prop_lines(props, "@IsOptional() "));
    s.push_str("}\n");
    out.counts.sym("class", 1);
    out.counts.sym("property", props);
    s
}

fn module_file(id: &Ident, out: &mut Out) -> String {
    let p = &id.pascal;
    let k = &id.kebab;
    out.counts.sym("class", 1);
    format!(
        "import {{ Module }} from '@nestjs/common';\nimport {{ {p}Controller }} from './{k}.controller';\nimport {{ {p}Service }} from './{k}.service';\nimport {{ {p}Repository }} from './{k}.repository';\n\n@Module({{\n  controllers: [{p}Controller],\n  providers: [{p}Service, {p}Repository],\n  exports: [{p}Service],\n}})\nexport class {p}Module {{}}\n"
    )
}

fn processor(id: &Ident, out: &mut Out) -> String {
    let p = &id.pascal;
    let k = &id.kebab;
    out.counts.processors += 1;
    out.counts.sym("class", 1);
    out.counts.sym("constructor", 1);
    out.counts.sym("property", 1);
    out.counts.sym("method", 3);
    let mut s = format!(
        "import {{ Processor, Process }} from '@nestjs/bull';\nimport {{ {p}Service }} from './{k}.service';\n\n@Processor('{k}-queue')\nexport class {p}Processor {{\n  constructor(private readonly service: {p}Service) {{}}\n"
    );
    for (n, job) in ["sync", "rebuild", "cleanup"].iter().enumerate() {
        s.push_str(&format!(
            "\n  @Process('{job}')\n  async handle{n}(job: unknown): Promise<unknown> {{\n    return this.service.findAll(String(job));\n  }}\n"
        ));
    }
    s.push_str("}\n");
    s
}

fn spec_file(id: &Ident, out: &mut Out) -> String {
    let p = &id.pascal;
    let k = &id.kebab;
    out.counts.spec_files += 1;
    format!(
        "import {{ {p}Service }} from './{k}.service';\nimport {{ {p}Repository }} from './{k}.repository';\n\ndescribe('{p}Service', () => {{\n  it('is defined', () => {{\n    expect(new {p}Service(new {p}Repository())).toBeDefined();\n  }});\n\n  it('finds rows', async () => {{\n    const service = new {p}Service(new {p}Repository());\n    expect(await service.findOne('x')).toBeUndefined();\n  }});\n}});\n"
    )
}

fn generated_file(id: &Ident, out: &mut Out) -> String {
    let p = &id.pascal;
    out.counts.generated_files += 1;
    out.counts.sym("interface", 1);
    out.counts.sym("property", 1);
    out.counts.sym("constant", 1);
    format!(
        "// @generated-by synth-repo\nexport interface {p}Generated {{\n  id: string;\n}}\n\nexport const {p}_GENERATED_VERSION = 1;\n"
    )
}

/// All files of one feature.
fn feature(plan: &Plan, mi: u32, fi: u32, rng: &mut Rng, out: &mut Out) {
    let id = ident(plan.seed, mi, fi);
    let base = id.dir.clone();
    let k = id.kebab.clone();
    let text = module_file(&id, out);
    out.file(format!("{base}/{k}.module.ts"), text);
    let text = controller(&id, rng, out);
    out.file(format!("{base}/{k}.controller.ts"), text);
    let text = service(plan, mi, fi, rng, out);
    out.file(format!("{base}/{k}.service.ts"), text);
    let text = repository(&id, out);
    out.file(format!("{base}/{k}.repository.ts"), text);
    let text = entity(plan, &id, rng, out);
    out.file(format!("{base}/{k}.entity.ts"), text);
    let text = dto(plan, &id, true, rng, out);
    out.file(format!("{base}/dto/create-{k}.dto.ts"), text);
    let text = dto(plan, &id, false, rng, out);
    out.file(format!("{base}/dto/update-{k}.dto.ts"), text);
    let text = spec_file(&id, out);
    out.file(format!("{base}/{k}.service.spec.ts"), text);
    if Plan::has_processor(fi) {
        let text = processor(&id, out);
        out.file(format!("{base}/{k}.processor.ts"), text);
    }
    if plan.has_generated(mi, fi) {
        let text = generated_file(&id, out);
        out.file(
            format!("src/{}/generated/{k}.generated.ts", id.mod_dir),
            text,
        );
    }
}

/// One module: every feature, in order, from a module-local RNG stream.
pub fn generate_module(plan: &Plan, mi: u32) -> Out {
    let mut rng = Rng::seeded(plan.seed, "module", u64::from(mi));
    let mut out = Out::default();
    for fi in 0..plan.spec.features_per_module {
        feature(plan, mi, fi, &mut rng, &mut out);
    }
    out
}

/// `src/shared`: the auth guard and the util files that hold the shared-call targets.
pub fn generate_shared(plan: &Plan) -> Out {
    let mut out = Out::default();
    out.file(
        "src/shared/auth.guard.ts".to_owned(),
        "import { Injectable } from '@nestjs/common';\n\n@Injectable()\nexport class AuthGuard {\n  canActivate(): boolean {\n    return true;\n  }\n}\n"
            .to_owned(),
    );
    out.counts.sym("class", 1);
    out.counts.sym("method", 1);
    for i in 0..plan.util_count() {
        let mut s = String::new();
        for j in 0..4 {
            s.push_str(&format!(
                "export function util{i}F{j}(value: string): string {{\n  return value.trim();\n}}\n\n"
            ));
        }
        out.counts.sym("function", 4);
        out.file(format!("src/shared/util-{i}.ts"), s);
    }
    out
}
