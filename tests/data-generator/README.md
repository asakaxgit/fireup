# Data Generator

This tool generates sample data for the Firestore emulator.

## Usage

### Generic Commands

The data generator uses a generic command interface. You can run any generator by specifying the generator name:

```bash
# With emulator (starts emulator, runs generator, exports data)
npm run generate -- 'node dist/run-generator.js <generator-name>'

# Without emulator (emulator must already be running)
npm run generate:no-emulator -- <generator-name>
```

Available generators:
- `level1` - Level 1 test data (two primitives per document)
- `level2` - Level 2 test data (two rows with single field)
- `level3` - Level 3 test data (two primitives + two rows)
- `main` - Main dataset with users and cities
- `tiny` - Minimal test data

Examples:
```bash
# Generate level2 data with emulator
npm run generate -- 'node dist/run-generator.js level2'

# Generate level3 data (emulator must be running)
npm run generate:no-emulator -- level3
```

### Option 1: Start emulator separately (Recommended for development)
```bash
# Terminal 1: Start the emulator with UI
npm run dev

# Terminal 2: Generate data (while emulator is running)
npm run generate:no-emulator -- level2
```

### Option 2: Generate data only (no UI)
```bash
npm run generate -- 'node dist/run-generator.js main'
```

## Viewing Data
- Open http://localhost:14000 in your browser
- Navigate to the Firestore tab
- You should see:
  - `users` collection with 2 documents (alovelace, aturing)
  - `cities` collection with 5 documents (SF, LA, DC, TOK, BJ)

## Adding New Generators

To add a new generator:

1. Create a new TypeScript file in `src/` (e.g., `src/level4_example.ts`)
2. Add the generator to the map in `src/run-generator.ts`:
   ```typescript
   const generatorMap: Record<string, string> = {
     'level1': './level1_two_primitives.js',
     'level2': './level2_two_rows.js',
     'level3': './level3_two_primitives_two_rows.js',
     'level4': './level4_example.js',  // Add your new generator here
     'main': './main.js',
     'tiny': './tiny.js',
   };
   ```
3. Run `npm run build` to compile
4. Run your generator with `npm run generate:no-emulator -- level4`

## Troubleshooting
- Make sure the emulator is running before using `generate:no-emulator`
- If data doesn't appear, check that the emulator UI is accessible at http://localhost:14000
- Data is exported to `../.firestore-data` when the emulator stops