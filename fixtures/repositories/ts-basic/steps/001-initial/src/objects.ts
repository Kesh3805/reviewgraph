export const handlers = {
  create() {},
  update: (id: string) => id,
  nested: {
    remove() {},
    deeper: { tooDeep() {} },
  },
  [computedKey]: 1,
  "quoted-key": function () {},
};
