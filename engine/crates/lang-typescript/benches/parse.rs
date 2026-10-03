#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use analysis_ir::{AnalyzerConfig, LanguageAnalyzer, SourceInput};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use lang_typescript::TypeScriptAnalyzer;
use review_core::location::{ContentHash, RepoPath};
use review_core::symbol::ModulePath;

const SERVICE: &str = r#"import { Injectable } from "@nestjs/common";
import { Repository } from "typeorm";

@Injectable()
export class OrdersService {
  constructor(private readonly repo: Repository<Order>, private readonly mailer: Mailer) {}

  async create(dto: CreateOrderDto): Promise<Order> {
    const order = this.repo.create(dto);
    if (!order.items.length) {
      throw new Error("empty order");
    }
    await this.repo.save(order);
    await this.mailer.send(order.email, "created");
    return order;
  }

  async findAll(): Promise<Order[]> {
    return this.repo.find({ where: { active: true } });
  }
}
"#;

fn bench_parse(c: &mut Criterion) {
    let source = SERVICE.repeat(1000);
    let path = RepoPath::new("src/orders.service.ts").unwrap();
    let input = SourceInput {
        module_path: ModulePath::of(&path),
        content_hash: ContentHash::of(source.as_bytes()),
        path,
        bytes: source.as_bytes(),
        is_generated: false,
    };
    let cfg = AnalyzerConfig {
        max_file_bytes: 64 * 1024 * 1024,
        parse_timeout_ms: 60_000,
        ..AnalyzerConfig::default()
    };
    let analyzer = TypeScriptAnalyzer::new();
    let mut group = c.benchmark_group("parse");
    group.throughput(Throughput::Bytes(source.len() as u64));
    group.bench_function("nest_service_x1000", |b| {
        b.iter(|| analyzer.analyze(&input, &cfg).unwrap());
    });
    group.finish();
}

criterion_group!(benches, bench_parse);
criterion_main!(benches);
