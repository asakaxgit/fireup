#!/usr/bin/env node

/**
 * Generic data generator runner.
 * Usage: node dist/run-generator.js <generator-name>
 * 
 * Available generators: level1, level2, level3, main, tiny
 */

const args = process.argv.slice(2);

if (args.length === 0) {
  console.error('Usage: node dist/run-generator.js <generator-name>');
  console.error('Available generators: level1, level2, level3, main, tiny');
  process.exit(1);
}

const generatorName: string = args[0]!;

// Map short names to actual module files
const generatorMap: Record<string, string> = {
  'level1': './level1_two_primitives.js',
  'level2': './level2_two_rows.js',
  'level3': './level3_two_primitives_two_rows.js',
  'main': './main.js',
  'tiny': './tiny.js',
};

const generatorPath: string | undefined = generatorMap[generatorName];

if (!generatorPath) {
  console.error(`Unknown generator: ${generatorName}`);
  console.error('Available generators: ' + Object.keys(generatorMap).join(', '));
  process.exit(1);
}

// Dynamically import and run the generator
console.log(`Running generator: ${generatorName}`);
await import(generatorPath);
