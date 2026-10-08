const computedKey = "computed";

export const routes = {
  list() {},
  create: (input: CreateInput) => input,
  nested: {
    remove() {},
    deeper: {
      tooDeep() {},
    },
  },
  [computedKey]: 1,
  "quoted-key": function () {},
  42: "numeric-key",
  [Symbol.iterator]() {},
};