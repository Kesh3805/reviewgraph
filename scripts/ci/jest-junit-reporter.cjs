'use strict';
// Minimal dependency-free JUnit reporter for Jest (CI-003). Output path: reporter option `output`.
const { mkdirSync, writeFileSync } = require('node:fs');
const { dirname } = require('node:path');

function esc(s) {
  return String(s)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function toXml(results) {
  const suites = results.testResults.map((suite) => {
    const cases = suite.testResults.map((t) => {
      const time = ((t.duration ?? 0) / 1000).toFixed(3);
      const head = `<testcase classname="${esc(suite.testFilePath)}" name="${esc(t.fullName)}" time="${time}"`;
      if (t.status === 'failed') {
        return `${head}><failure message="failed">${esc((t.failureMessages || []).join('\n'))}</failure></testcase>`;
      }
      if (t.status === 'pending' || t.status === 'skipped' || t.status === 'todo') {
        return `${head}><skipped/></testcase>`;
      }
      return `${head}/>`;
    });
    const failures = suite.testResults.filter((t) => t.status === 'failed').length;
    return `<testsuite name="${esc(suite.testFilePath)}" tests="${suite.testResults.length}" failures="${failures}">${cases.join('')}</testsuite>`;
  });
  return `<?xml version="1.0" encoding="UTF-8"?>\n<testsuites tests="${results.numTotalTests}" failures="${results.numFailedTests}">${suites.join('')}</testsuites>\n`;
}

class JunitReporter {
  constructor(_globalConfig, options) {
    this.output = (options && options.output) || 'junit.xml';
  }

  onRunComplete(_contexts, results) {
    const out = this.output;
    mkdirSync(dirname(out), { recursive: true });
    writeFileSync(out, toXml(results));
  }
}

module.exports = JunitReporter;
