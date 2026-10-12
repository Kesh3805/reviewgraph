#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use analysis_ir::{FrameworkFactKind, IrFrameworkFact};
use common::{analyze, battr, facts_of, iattr, sattr, strs};

const HEAD: &str = "import { Processor, WorkerHost, InjectQueue, BullModule, OnWorkerEvent } from '@nestjs/bullmq';\n\
                    import { Injectable, Module } from '@nestjs/common';\n\
                    import { Queue, Worker, FlowProducer, Job } from 'bullmq';\n";

fn of(kind: FrameworkFactKind, body: &str) -> Vec<IrFrameworkFact> {
    facts_of("src/queue/mail.ts", &format!("{HEAD}{body}"), kind)
}

#[test]
fn processor_string_and_object_forms() {
    let facts = of(
        FrameworkFactKind::QueueConsumer,
        "@Processor('mail') class A extends WorkerHost {}\n\
         @Processor({ name: 'sms', concurrency: 5 }) class B extends WorkerHost {}\n",
    );
    assert_eq!(facts.len(), 2);
    assert_eq!(sattr(&facts[0], "queue").as_deref(), Some("mail"));
    assert_eq!(sattr(&facts[0], "style").as_deref(), Some("worker_host"));
    assert_eq!(sattr(&facts[1], "queue").as_deref(), Some("sms"));
    assert_eq!(iattr(&facts[1], "concurrency"), Some(5));
    assert_eq!(sattr(&facts[1], "symbol").as_deref(), Some("B"));
}

#[test]
fn worker_host_process_switch_cases_to_handlers() {
    let facts = of(
        FrameworkFactKind::QueueJobHandler,
        "@Processor('mail') export class MailProcessor extends WorkerHost {\n\
           async process(job: Job) {\n\
             switch (job.name) {\n\
               case 'welcome': return 1;\n\
               case 'reset': return 2;\n\
             }\n\
           }\n\
         }\n",
    );
    let jobs: Vec<String> = facts.iter().filter_map(|f| sattr(f, "job")).collect();
    assert_eq!(jobs, vec!["welcome", "reset"]);
    assert!(facts
        .iter()
        .all(|f| sattr(f, "via").as_deref() == Some("switch_case")));
    assert_eq!(
        sattr(&facts[0], "symbol").as_deref(),
        Some("MailProcessor.process")
    );
    assert_eq!(sattr(&facts[0], "queue").as_deref(), Some("mail"));
}

#[test]
fn worker_host_without_switch_star_handler() {
    let facts = of(
        FrameworkFactKind::QueueJobHandler,
        "@Processor('mail') class P extends WorkerHost { async process(job: Job) { return job.data; } }\n",
    );
    assert_eq!(facts.len(), 1);
    assert_eq!(sattr(&facts[0], "job").as_deref(), Some("*"));
    assert_eq!(
        sattr(&facts[0], "via").as_deref(),
        Some("worker_host_process")
    );
}

#[test]
fn legacy_process_decorator_named_and_default() {
    let facts = facts_of(
        "src/queue/legacy.ts",
        "import { Processor, Process } from '@nestjs/bull';\n\
         @Processor('q') class L {\n\
           @Process('named') a() {}\n\
           @Process() b() {}\n\
           @Process({ name: 'obj' }) c() {}\n\
         }\n",
        FrameworkFactKind::QueueJobHandler,
    );
    let jobs: Vec<String> = facts.iter().filter_map(|f| sattr(f, "job")).collect();
    assert_eq!(jobs, vec!["named", "*", "obj"]);
    let consumer = facts_of(
        "src/queue/legacy.ts",
        "import { Processor } from '@nestjs/bull';\n@Processor('q') class L {}\n",
        FrameworkFactKind::QueueConsumer,
    );
    assert_eq!(sattr(&consumer[0], "style").as_deref(), Some("legacy_bull"));
}

#[test]
fn on_worker_event_not_a_job_handler() {
    let facts = of(
        FrameworkFactKind::QueueJobHandler,
        "@Processor('mail') class P extends WorkerHost {\n\
           @OnWorkerEvent('completed') onDone() {}\n\
         }\n",
    );
    let events: Vec<&IrFrameworkFact> = facts
        .iter()
        .filter(|f| sattr(f, "event").is_some())
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(sattr(events[0], "event").as_deref(), Some("completed"));
    assert!(sattr(events[0], "job").is_none());
    assert!(facts
        .iter()
        .all(|f| sattr(f, "via").as_deref() != Some("process_decorator")));
}

#[test]
fn register_queue_single_and_array_and_async() {
    let facts = of(
        FrameworkFactKind::QueueRegistration,
        "@Module({ imports: [\n\
           BullModule.registerQueue({ name: 'a' }, { name: 'b', defaultJobOptions: { jobId: 'x' } }),\n\
           BullModule.registerQueueAsync({ name: 'c', useFactory: () => ({}) }),\n\
         ] }) class M {}\n",
    );
    let queues: Vec<String> = facts.iter().filter_map(|f| sattr(f, "queue")).collect();
    assert_eq!(queues, vec!["a", "b", "c"]);
    assert_eq!(battr(&facts[1], "default_job_id_set"), Some(true));
    assert_eq!(battr(&facts[0], "default_job_id_set"), Some(false));
    assert!(strs(&facts[2], "options_keys").contains(&"useFactory".to_owned()));
    assert_eq!(sattr(&facts[0], "via").as_deref(), Some("register_queue"));
}

#[test]
fn new_queue_and_new_worker() {
    let source = "import { Queue, Worker } from 'bullmq';\n\
                  const q = new Queue('reports');\n\
                  const w = new Worker('reports', async (job) => {});\n";
    let regs = facts_of("src/q.ts", source, FrameworkFactKind::QueueRegistration);
    let vias: Vec<String> = regs.iter().filter_map(|f| sattr(f, "via")).collect();
    assert_eq!(vias, vec!["new_queue", "new_worker"]);
    let consumers = facts_of("src/q.ts", source, FrameworkFactKind::QueueConsumer);
    assert_eq!(consumers.len(), 1);
    assert_eq!(
        sattr(&consumers[0], "style").as_deref(),
        Some("worker_instance")
    );
}

const PRODUCER: &str = "@Injectable() class P {\n\
    constructor(@InjectQueue('mail') private readonly queue: Queue) {}\n";

#[test]
fn producer_add_via_inject_queue() {
    let facts = of(
        FrameworkFactKind::QueueProducer,
        &format!(
            "{PRODUCER} async send() {{ await this.queue.add('welcome', {{ id: 1 }}, {{ jobId: 'x' }}); }} }}\n"
        ),
    );
    assert_eq!(facts.len(), 1);
    assert_eq!(sattr(&facts[0], "queue").as_deref(), Some("mail"));
    assert_eq!(sattr(&facts[0], "job").as_deref(), Some("welcome"));
    assert_eq!(sattr(&facts[0], "method").as_deref(), Some("add"));
    assert_eq!(battr(&facts[0], "job_id_set"), Some(true));
    assert_eq!(
        sattr(&facts[0], "receiver_declared_type").as_deref(),
        Some("Queue")
    );
    assert!((facts[0].confidence - 0.9).abs() < 1e-6);
}

#[test]
fn producer_add_bulk_elements() {
    let facts = of(
        FrameworkFactKind::QueueProducer,
        &format!(
            "{PRODUCER} async send() {{ await this.queue.addBulk([{{ name: 'a', data: {{}} }}, {{ name: 'b', data: {{}}, opts: {{ delay: 5 }} }}]); }} }}\n"
        ),
    );
    assert_eq!(facts.len(), 2);
    assert_eq!(sattr(&facts[0], "method").as_deref(), Some("add_bulk"));
    assert_eq!(sattr(&facts[1], "job").as_deref(), Some("b"));
    assert_eq!(battr(&facts[1], "delay_set"), Some(true));
    assert_eq!(battr(&facts[0], "delay_set"), Some(false));
}

#[test]
fn job_id_option_flag() {
    let facts = of(
        FrameworkFactKind::QueueProducer,
        &format!(
            "{PRODUCER} async send() {{ await this.queue.add('a', {{}}); await this.queue.add('b', {{}}, {{ deduplication: {{ id: 'z' }} }}); }} }}\n"
        ),
    );
    assert_eq!(battr(&facts[0], "job_id_set"), Some(false));
    assert_eq!(battr(&facts[1], "job_id_set"), Some(true));
}

#[test]
fn queue_name_from_same_file_const_and_enum() {
    let facts = of(
        FrameworkFactKind::QueueConsumer,
        "const MAIL = 'mail';\nenum Q { SMS = 'sms' }\n\
         @Processor(MAIL) class A extends WorkerHost {}\n\
         @Processor(Q.SMS) class B extends WorkerHost {}\n",
    );
    assert_eq!(sattr(&facts[0], "queue").as_deref(), Some("mail"));
    assert_eq!(sattr(&facts[1], "queue").as_deref(), Some("sms"));
    assert_eq!(battr(&facts[0], "queue_dynamic"), Some(false));
}

#[test]
fn dynamic_queue_name_low_confidence() {
    let facts = of(
        FrameworkFactKind::QueueConsumer,
        "@Processor(getName()) class A extends WorkerHost {}\n",
    );
    assert_eq!(battr(&facts[0], "queue_dynamic"), Some(true));
    assert!(sattr(&facts[0], "queue").unwrap().starts_with('{'));
    assert!(facts[0].confidence <= 0.5);
}

#[test]
fn flow_producer_children_depth_cap() {
    let facts = of(
        FrameworkFactKind::QueueProducer,
        "@Injectable() class F {\n\
           constructor(private flow: FlowProducer) {}\n\
           async run() {\n\
             await this.flow.add({ name: 'p', queueName: 'a', children: [\n\
               { name: 'c1', queueName: 'b', children: [\n\
                 { name: 'c2', queueName: 'c', children: [ { name: 'c3', queueName: 'd' } ] } ] } ] });\n\
           }\n\
         }\n",
    );
    let jobs: Vec<String> = facts.iter().filter_map(|f| sattr(f, "job")).collect();
    assert_eq!(jobs, vec!["p", "c1", "c2"]);
    assert!(facts
        .iter()
        .all(|f| sattr(f, "method").as_deref() == Some("flow_add")));
    assert_eq!(sattr(&facts[1], "queue").as_deref(), Some("b"));
}

#[test]
fn non_bullmq_queue_class_ignored() {
    let unit = analyze(
        "src/a.ts",
        "import { Queue } from './my-queue';\nclass A { constructor(private q: Queue) {} f() { this.q.add('x'); } }\n",
    );
    assert!(unit.framework.is_empty());
}

#[test]
fn unbound_processor_decorator_lower_confidence() {
    let facts = facts_of(
        "src/a.ts",
        "import { WorkerHost } from '@nestjs/bullmq';\n@Processor('q') class A extends WorkerHost {}\n",
        FrameworkFactKind::QueueConsumer,
    );
    assert!(facts[0].confidence < 0.9);
}

#[test]
fn type_only_queue_receiver_is_unresolved() {
    let facts = facts_of(
        "src/a.ts",
        "import { Queue } from 'bullmq';\nclass A { constructor(private queue: Queue) {} f() { this.queue.add('x', {}); } }\n",
        FrameworkFactKind::QueueProducer,
    );
    assert_eq!(facts.len(), 1);
    assert_eq!(battr(&facts[0], "queue_dynamic"), Some(true));
    assert!(facts[0].confidence <= 0.5);
}
