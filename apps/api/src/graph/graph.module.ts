import { Module } from '@nestjs/common';
import { EngineClient } from './engine.client';
import { GraphController } from './graph.controller';
import { GRAPH_SCOPE, PgGraphScope } from './graph-scope';
import { GraphService } from './graph.service';

@Module({
  controllers: [GraphController],
  providers: [EngineClient, GraphService, { provide: GRAPH_SCOPE, useClass: PgGraphScope }],
  exports: [EngineClient],
})
export class GraphModule {}
