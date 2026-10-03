import { NestFactory } from "@nestjs/core";
import { sharedValue } from "@acme/shared";
import { Db } from "@acme/db";

async function bootstrap() {
  const app = await NestFactory.create({});
  console.log(sharedValue, Db);
  await app.listen(3000);
}
bootstrap();
