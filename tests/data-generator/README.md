# Data Generator

This tool generates sample data for the Firestore emulator.

## Usage

### Option 1: Generate data and view in UI (Recommended)
```bash
npm run generate-with-ui
```
This will:
1. Start the emulator
2. Generate the data
3. Keep the emulator running so you can view data at http://localhost:14000
4. Export data when you stop the emulator

### Option 2: Start emulator separately
```bash
# Terminal 1: Start the emulator with UI
npm run dev

# Terminal 2: Generate data (while emulator is running)
npm run generate-without-emulator
```

### Option 3: Generate data only (no UI)
```bash
npm run generate
```

## Viewing Data
- Open http://localhost:14000 in your browser
- Navigate to the Firestore tab
- You should see:
  - `users` collection with 2 documents (alovelace, aturing)
  - `cities` collection with 5 documents (SF, LA, DC, TOK, BJ)

## Troubleshooting
- Make sure the emulator is running before using `generate-without-emulator`
- If data doesn't appear, check that the emulator UI is accessible at http://localhost:14000
- Data is exported to `../.firestore-data` when the emulator stops