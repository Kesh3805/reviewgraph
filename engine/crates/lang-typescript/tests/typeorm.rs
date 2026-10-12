#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use analysis_ir::{AttrValue, FrameworkFactKind, IrFrameworkFact};
use common::{battr, facts_of, iattr, sattr};

const HEAD: &str = "import { Entity, ViewEntity, ChildEntity, Column, PrimaryColumn, \
                    PrimaryGeneratedColumn, CreateDateColumn, OneToMany, ManyToOne, JoinColumn, \
                    Repository, EntityManager, DataSource } from 'typeorm';\n\
                    import { InjectRepository } from '@nestjs/typeorm';\n\
                    import { Injectable } from '@nestjs/common';\n";

fn of(kind: FrameworkFactKind, body: &str) -> Vec<IrFrameworkFact> {
    facts_of("src/db/thing.ts", &format!("{HEAD}{body}"), kind)
}

fn entities(body: &str) -> Vec<IrFrameworkFact> {
    of(FrameworkFactKind::OrmEntity, body)
}

fn access(body: &str) -> Vec<IrFrameworkFact> {
    of(FrameworkFactKind::OrmAccess, body)
}

#[test]
fn entity_explicit_name_and_schema() {
    let facts = entities(
        "@Entity('users', { schema: 'app' }) class User {}\n\
         @Entity({ name: 'accounts', schema: 'billing' }) class Account {}\n",
    );
    assert_eq!(sattr(&facts[0], "table").as_deref(), Some("users"));
    assert_eq!(sattr(&facts[0], "schema").as_deref(), Some("app"));
    assert_eq!(battr(&facts[0], "table_inferred"), Some(false));
    assert_eq!(sattr(&facts[1], "table").as_deref(), Some("accounts"));
    assert_eq!(sattr(&facts[1], "schema").as_deref(), Some("billing"));
    assert!(facts[0].confidence > 0.9);
}

#[test]
fn entity_default_snake_case_inferred() {
    let facts = entities("@Entity() class UserAccount {}\n");
    assert_eq!(sattr(&facts[0], "table").as_deref(), Some("user_account"));
    assert_eq!(battr(&facts[0], "table_inferred"), Some(true));
    assert!(facts[0].confidence <= 0.7 + 1e-6);
    assert_eq!(sattr(&facts[0], "symbol").as_deref(), Some("UserAccount"));
}

#[test]
fn view_and_child_entity() {
    let facts = entities(
        "@ViewEntity({ name: 'v_x', expression: 'select 1' }) class V {}\n\
         @Entity() class User {}\n\
         @ChildEntity() class Admin extends User {}\n",
    );
    assert_eq!(sattr(&facts[0], "kind").as_deref(), Some("view"));
    assert_eq!(sattr(&facts[0], "table").as_deref(), Some("v_x"));
    assert_eq!(sattr(&facts[2], "kind").as_deref(), Some("child"));
    assert_eq!(sattr(&facts[2], "table").as_deref(), Some("user"));
    assert_eq!(sattr(&facts[2], "extends").as_deref(), Some("User"));
    assert!(!format!("{facts:?}").contains("select 1"));
}

#[test]
fn column_kinds_and_options() {
    let facts = of(
        FrameworkFactKind::OrmColumn,
        "@Entity() class User {\n\
           @PrimaryGeneratedColumn('uuid') id: string;\n\
           @Column({ type: 'varchar', length: 255, nullable: true, unique: true, name: 'mail' }) email: string;\n\
           @CreateDateColumn() createdAt: Date;\n\
           @Column('text', { default: 'hunter2' }) bio: string;\n\
         }\n",
    );
    assert_eq!(facts.len(), 4);
    assert_eq!(sattr(&facts[0], "kind").as_deref(), Some("primary"));
    assert_eq!(battr(&facts[0], "primary"), Some(true));
    assert_eq!(battr(&facts[0], "generated"), Some(true));
    assert_eq!(sattr(&facts[1], "column").as_deref(), Some("mail"));
    assert_eq!(sattr(&facts[1], "column_type").as_deref(), Some("varchar"));
    assert_eq!(iattr(&facts[1], "length"), Some(255));
    assert_eq!(battr(&facts[1], "nullable"), Some(true));
    assert_eq!(battr(&facts[1], "unique"), Some(true));
    assert_eq!(sattr(&facts[2], "kind").as_deref(), Some("created_at"));
    assert_eq!(sattr(&facts[3], "column_type").as_deref(), Some("text"));
    assert!(!format!("{facts:?}").contains("hunter2"));
    assert_eq!(sattr(&facts[1], "entity").as_deref(), Some("User"));
}

#[test]
fn relations_with_inverse_and_join() {
    let facts = of(
        FrameworkFactKind::OrmRelation,
        "@Entity() class User {\n\
           @OneToMany(() => Order, (order) => order.user, { cascade: true, eager: true }) orders: Order[];\n\
         }\n\
         @Entity() class Order {\n\
           @ManyToOne(() => User, (u) => u.orders, { nullable: false }) @JoinColumn({ name: 'user_id' }) user: User;\n\
         }\n",
    );
    assert_eq!(facts.len(), 2);
    assert_eq!(sattr(&facts[0], "relation").as_deref(), Some("one_to_many"));
    assert_eq!(sattr(&facts[0], "target").as_deref(), Some("Order"));
    assert_eq!(
        sattr(&facts[0], "inverse_property").as_deref(),
        Some("user")
    );
    assert_eq!(facts[0].attrs.get("cascade"), Some(&AttrValue::Bool(true)));
    assert_eq!(battr(&facts[0], "eager"), Some(true));
    assert_eq!(sattr(&facts[1], "relation").as_deref(), Some("many_to_one"));
    assert_eq!(
        sattr(&facts[1], "join_column_name").as_deref(),
        Some("user_id")
    );
    assert_eq!(battr(&facts[1], "nullable"), Some(false));
}

#[test]
fn inherited_base_entity_columns_noted() {
    let columns = of(
        FrameworkFactKind::OrmColumn,
        "class Base { @PrimaryColumn() id: number; }\n@Entity() class User extends Base {}\n",
    );
    assert_eq!(sattr(&columns[0], "entity").as_deref(), Some("Base"));
    let ents = entities(
        "class Base { @PrimaryColumn() id: number; }\n@Entity() class User extends Base {}\n",
    );
    assert_eq!(sattr(&ents[0], "extends").as_deref(), Some("Base"));
}

const SERVICE_HEAD: &str = "@Injectable() class S {\n\
    constructor(private readonly repo: Repository<User>, private manager: EntityManager, \
    private dataSource: DataSource) {}\n";

#[test]
fn repository_save_write_with_entity_from_generic() {
    let facts = access(&format!(
        "{SERVICE_HEAD} async f(u: User) {{ await this.repo.save(u); }} }}\n"
    ));
    assert_eq!(facts.len(), 1);
    assert_eq!(sattr(&facts[0], "entity").as_deref(), Some("User"));
    assert_eq!(sattr(&facts[0], "op").as_deref(), Some("write"));
    assert_eq!(sattr(&facts[0], "via").as_deref(), Some("repository"));
    assert_eq!(sattr(&facts[0], "symbol").as_deref(), Some("S.f"));
    assert!((facts[0].confidence - 0.9).abs() < 1e-6);
}

#[test]
fn find_read_ops() {
    let facts = access(&format!(
        "{SERVICE_HEAD} async f() {{ await this.repo.find(); await this.repo.findOneBy({{ id: 1 }}); await this.repo.count(); }} }}\n"
    ));
    assert_eq!(facts.len(), 3);
    assert!(facts
        .iter()
        .all(|f| sattr(f, "op").as_deref() == Some("read")));
}

#[test]
fn inject_repository_token_resolves_entity() {
    let facts = access(
        "@Injectable() class S {\n\
           constructor(@InjectRepository(User) private readonly repo: Repository) {}\n\
           async f(u: User) { await this.repo.save(u); }\n\
         }\n",
    );
    assert_eq!(facts.len(), 1);
    assert_eq!(sattr(&facts[0], "entity").as_deref(), Some("User"));
    assert!((facts[0].confidence - 0.9).abs() < 1e-6);
}

#[test]
fn manager_save_first_arg_entity() {
    let facts = access(&format!(
        "{SERVICE_HEAD} async f(u: any) {{ await this.manager.save(User, u); }} }}\n"
    ));
    assert_eq!(sattr(&facts[0], "entity").as_deref(), Some("User"));
    assert_eq!(sattr(&facts[0], "via").as_deref(), Some("manager"));
    assert!((facts[0].confidence - 0.85).abs() < 1e-6);
}

#[test]
fn get_repository_call() {
    let facts = access(&format!(
        "{SERVICE_HEAD} async f(u: any) {{ await this.dataSource.getRepository(User).save(u); }} }}\n"
    ));
    assert_eq!(facts.len(), 1);
    assert_eq!(sattr(&facts[0], "entity").as_deref(), Some("User"));
    assert_eq!(sattr(&facts[0], "op").as_deref(), Some("write"));
}

#[test]
fn query_builder_insert_update_delete_chain() {
    let facts = access(&format!(
        "{SERVICE_HEAD} async f(v: any) {{ await this.dataSource.createQueryBuilder().insert().into(User).values(v).execute(); }} }}\n"
    ));
    assert_eq!(facts.len(), 1);
    assert_eq!(sattr(&facts[0], "op").as_deref(), Some("write"));
    assert_eq!(sattr(&facts[0], "via").as_deref(), Some("query_builder"));
    assert_eq!(sattr(&facts[0], "entity").as_deref(), Some("User"));
}

#[test]
fn query_builder_select_read() {
    let facts = access(&format!(
        "{SERVICE_HEAD} async f() {{ return this.repo.createQueryBuilder('u').where('u.x = :x').getMany(); }} }}\n"
    ));
    assert_eq!(facts.len(), 1);
    assert_eq!(sattr(&facts[0], "op").as_deref(), Some("read"));
    assert_eq!(sattr(&facts[0], "entity").as_deref(), Some("User"));
}

#[test]
fn raw_query_verbs_and_table_hint() {
    let facts = access(&format!(
        "{SERVICE_HEAD} async f(id: number) {{\n\
           await this.dataSource.query('UPDATE users SET a = 1 WHERE id = $1', [id]);\n\
           await this.dataSource.query('SELECT * FROM orders o JOIN users u ON u.id = o.uid');\n\
         }} }}\n"
    ));
    assert_eq!(facts.len(), 2);
    assert_eq!(sattr(&facts[0], "op").as_deref(), Some("write"));
    assert_eq!(sattr(&facts[0], "table_hint").as_deref(), Some("users"));
    assert_eq!(sattr(&facts[0], "via").as_deref(), Some("raw_sql"));
    assert_eq!(sattr(&facts[1], "op").as_deref(), Some("read"));
    assert_eq!(sattr(&facts[1], "table_hint").as_deref(), Some("orders"));
}

#[test]
fn sql_text_not_stored() {
    let facts = access(&format!(
        "{SERVICE_HEAD} async f() {{ await this.dataSource.query('UPDATE users SET secret_col = 42'); }} }}\n"
    ));
    let dump = format!("{facts:?}");
    assert!(!dump.contains("secret_col"));
    assert!(!dump.contains("SET"));
}

#[test]
fn non_db_save_ignored() {
    let facts = access(
        "@Injectable() class S { constructor(private fileStore: FileStore) {} async f(x: any) { await this.fileStore.save(x); } }\n",
    );
    assert!(facts.is_empty());
}

#[test]
fn unresolved_entity_low_confidence() {
    let facts = access(
        "@Injectable() class S { constructor(private repo: Repository) {} async f(x: any) { await this.repo.save(x); } }\n",
    );
    assert_eq!(facts.len(), 1);
    assert!(!facts[0].attrs.contains_key("entity"));
    assert!(facts[0].confidence <= 0.5);
}

#[test]
fn transaction_flag_propagated() {
    let facts = access(&format!(
        "{SERVICE_HEAD} async f(u: any) {{\n\
           await this.dataSource.transaction(async (manager: EntityManager) => {{ await manager.save(User, u); }});\n\
           await this.repo.save(u);\n\
         }} }}\n"
    ));
    assert_eq!(facts.len(), 2);
    assert_eq!(battr(&facts[0], "in_transaction"), Some(true));
    assert_eq!(battr(&facts[1], "in_transaction"), Some(false));
}
