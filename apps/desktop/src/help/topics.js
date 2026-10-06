// The help: every topic, in reading order.
//
// Bodies use a small markup, rendered by ../views/help.js into DOM nodes
// (never innerHTML):
//
//   blank line     new block           ## Heading    a subheading
//   1. …           numbered steps      - …           bullets
//   > …            a tip               ! …           a warning
//   **bold**       «Label» a control or label as it appears in the app
//   [[topic-id]] or [[topic-id|text]]  a link to another help topic
//   {{view|text}}                      a link to a screen (holdings, markets…)
//
// Keep labels in «» exactly as the app shows them, so searching for what is
// on screen finds the topic that explains it.

export const SECTIONS = [
  ["start", "Getting started"],
  ["items", "Adding and editing items"],
  ["values", "Values and prices"],
  ["organize", "Finding and organizing"],
  ["files", "Photos and documents"],
  ["care", "Looking after things"],
  ["overview", "Overview and markets"],
  ["reports", "Reports, claims and spreadsheets"],
  ["safety", "Backups and security"],
  ["reference", "Reference"],
];

export const TOPICS = [
  // ------------------------------------------------------------ getting started
  {
    id: "welcome",
    section: "start",
    title: "What Asset Manager does",
    summary: "A private, encrypted catalog of everything you own and what it is worth.",
    keywords: "about introduction overview purpose offline private",
    body: `Asset Manager keeps a catalog of what you own — precious metals, coins, cards, comics, watches, jewelry, wine, crypto, electronics, vehicles, property and anything else — with photos, receipts, where each thing is kept, and what it is worth.

Everything lives in one encrypted **vault** on this computer. There is no account, no cloud and no tracking. Nothing leaves the computer unless you turn on a price feed or a balance lookup, and even then only what that feature needs (see [[network]]).

Use it to:

- Know what you have and what it is worth today, and how that changed over time.
- Prove ownership and value after a loss: print an insurance inventory or prepare a claim for just the items lost.
- Keep the paperwork — receipts, appraisals, certificates, warranties — with the item it belongs to.
- Track loans, repairs, services and warranties, and check a room or safe against the catalog.

Values are honest: market prices where they exist, your own figures everywhere else, and anything without a value is counted as unknown, never as zero. See [[values-explained]].

New here? Start with [[create-vault]], then [[first-item]].`,
  },
  {
    id: "create-vault",
    section: "start",
    title: "Creating your vault",
    summary: "Choose a passphrase; the vault is created and encrypted with it.",
    keywords: "setup new vault passphrase password first run welcome create",
    body: `The first time you open the app it offers «Create a new vault» or «Restore from a backup». To start fresh:

1. Choose «Create a new vault».
2. Type a passphrase in «Passphrase» and again in «Confirm passphrase». It must be at least 12 characters.
3. Choose «Create vault». Deriving the key takes a moment on purpose — it is what makes guessing slow.
4. Save the recovery key you are shown next — see [[recovery-key]].

A meter rates the passphrase as you type. Several unrelated words ("copper lantern violin harbor") are easier to remember than symbols and far harder to guess.

! There is no password reset. If you lose both the passphrase and the recovery key, the catalog cannot be recovered — by anyone, including the app's author.

Moving from another computer instead? See [[restore]].`,
  },
  {
    id: "recovery-key",
    section: "start",
    title: "Your recovery key",
    summary: "A second way to open the vault if you forget the passphrase. Shown once.",
    keywords: "recovery key sheet print qr code fingerprint backup key forgot lost",
    body: `When a vault is created you get a **recovery key**: a long code that opens the vault without the passphrase. It is shown once.

1. Keep it with «Print» (a sheet with the key and a QR code), «Save as text», or «Copy».
2. Tick «I have stored this key somewhere safe and offline».
3. Type the last 6 characters of the key to confirm you have it right, then continue.

Store it away from the computer: a printed sheet in a drawer or safe, or a USB stick kept offline. Anyone holding it can open the vault, so treat it like the passphrase.

The **fingerprint** printed with the key identifies which sheet belongs to this vault; it reveals nothing about the key itself. You can see the current fingerprint in {{settings|Settings}} → «Security».

> If the vault locked or the app closed before you confirmed the key, the app reminds you after you next unlock and offers a new one — the unconfirmed key cannot be shown again.

To replace a key that may have been seen, see [[new-recovery-key]]. If you have forgotten your passphrase, see [[forgot-passphrase]].`,
  },
  {
    id: "unlocking",
    section: "start",
    title: "Unlocking and locking",
    summary: "Open the vault with your passphrase or recovery key; it locks itself when idle.",
    keywords: "unlock lock login sign in auto-lock idle timeout inactivity session passphrase",
    body: `## Unlocking

Type your passphrase on the unlock screen and press «Unlock». To use the recovery key instead, choose «Use the recovery key instead» and type the key from your sheet.

## Locking

Choose «Lock vault» at the bottom of the sidebar at any time. The vault also locks itself after a period of inactivity — 30 minutes unless you change it in {{settings|Settings}} → «Security» → «Lock after inactivity» (5 minutes to 2 hours).

A minute before it locks, a countdown appears above «Lock vault»; moving the mouse or pressing a key keeps the vault open.

When the vault locks, everything decrypted is dropped from the screen and from memory — open dialogs, lists, photos, even a half-finished edit. You are returned to the unlock screen.

> Reading and scrolling count as activity, so the vault will not lock while you are using it.`,
  },
  {
    id: "first-item",
    section: "start",
    title: "Adding your first item",
    summary: "A quick walk through adding something to the catalog.",
    keywords: "first item quick start tutorial begin add asset",
    body: `1. Choose «Add asset» (top right of {{overview|Overview}} or {{holdings|Holdings}}).
2. Pick what it is — «Watch», «Comic book», «Precious metal», «Vehicle»… Type in «Find a kind» to search. «Anything else» takes any item.
3. Fill in what you know. Starred fields are needed to name the item; everything else is optional and can be added later.
4. Under «Value & insurance», enter what it would sell for today if you know it. Leave it blank if you do not — it will be counted as unknown, not zero.
5. Choose «Add to catalog». To enter several similar things, choose «Add and start another»: the next form starts from this one's details.

The new item opens on its own page, where you can add photos, documents and more. See [[asset-page]].

> Already keep a spreadsheet? Import it instead — see [[spreadsheet-import]].`,
  },
  {
    id: "getting-around",
    section: "start",
    title: "Finding your way around",
    summary: "The sidebar, what each screen is for, and shortcuts.",
    keywords: "navigation sidebar menu screens tour layout",
    body: `The sidebar on the left holds every screen:

- {{overview|Overview}} — total value, value over time, what needs attention, and what is coming up. See [[overview]].
- {{holdings|Holdings}} — the whole catalog: search, filter, sort, select many, and bulk changes. See [[holdings-list]].
- {{wishlist|Wishlist}} — things you want, kept apart from what you own. See [[wishlist]].
- {{markets|Markets}} — metal spot prices, crypto prices, exchange rates and the melt calculator. See [[spot-prices]].
- {{reports|Reports}} — insurance inventory, claims, and spreadsheet import and export. See [[insurance-report]].
- {{settings|Settings}} — security, backups, trash, your own item types, price feeds and currency.
- {{help|Help}} — this guide.

Click any item in a list to open its page.

## Shortcuts

- «/» jumps to the search box on the current screen (or opens Holdings and searches there).
- «?» opens this help.

More in [[shortcuts]].`,
  },

  // ------------------------------------------------------------ items
  {
    id: "add-item",
    section: "items",
    title: "Adding an item",
    summary: "Pick a kind, fill in the details, and add it.",
    keywords: "add asset new item create form kind picker name starred fields add and start another",
    body: `Choose «Add asset» on {{overview|Overview}} or {{holdings|Holdings}}.

## Choosing a kind

The picker groups kinds under «Investments», «Valuables», «Home», «Vehicles & property», «Collectibles», «Firearms» and «Other», plus «Your types» if you have made any ([[custom-types]]). Type in «Find a kind — car, watch, comic…» to filter. If nothing fits, «Anything else» takes any item.

The kind decides which details the form asks for — a watch has a reference and serial, a vehicle a VIN, a comic a grade. See [[kinds]].

## Filling in the form

- **Starred fields** are what the item's name is built from. Leave «Name» blank and it is named for you — the form shows «Will be listed as …» as you type.
- «Quantity» and «Unit» — for things held in numbers: 20 coins, 3 bottles, 0.5 BTC.
- «Ownership» — «Acquired on», «Price paid (total)» for everything held, and «Acquired from».
- «Value & insurance» — what it would sell for today, and an insured value if it has one.
- «Notes & tags» — free notes, «Tags», a «Storage location», and «Remind me to revalue».

Prices can be typed as you would write them: «$1,299.50», «1299.5». Leave a price blank if you do not know it — never enter 0 for unknown.

## Saving

«Add to catalog» saves and opens the new item. «Add and start another» saves and starts a new form from the same details — handy for a run of similar coins or cards.`,
  },
  {
    id: "kinds",
    section: "items",
    title: "What you can catalog",
    summary: "Every built-in kind and the details each one asks for.",
    keywords: "kinds types categories watch jewelry art wine electronics furniture vehicle boat property land comic card coin stamp books toys firearm ammunition cash",
    body: `## Investments

- «Precious metal» — bullion, rounds and bullion coins, valued from spot. See [[metals]].
- «Cryptocurrency» — coins and tokens, priced by coin ID. See [[crypto]].
- «Stock, bond or fund» — held directly or in an account, valued by hand.

## Valuables

«Watch» (reference, serial, box & papers), «Jewelry» (metal, stones, appraisal), «Art & prints» (artist, medium, edition), «Fashion & accessories», «Instrument» (maker, model, serial), «Wine & spirits» (producer, vintage, bottle size).

## Home

«Electronics», «Furniture», «Appliance», «Tools & equipment», «Sports & outdoor».

## Vehicles & property

«Vehicle» (VIN, registration), «Boat» (hull ID, registration, length), «Home or building» and «Land» (parcel, acreage, your share). See [[vehicles-property]].

## Collectibles

«Comic book», «Sports card», «TCG card» (Magic, Pokémon, Yu-Gi-Oh), «Collector coin», «Stamp», «Memorabilia», «Books», «Toys & figures», «Sealed product», «Video game», «Vinyl record». Graded items record the grader, grade and certificate number — see [[graded]].

## Firearms

«Firearm» (make, model, caliber, serial), «Ammunition» (caliber, load, rounds held), «Optics & accessories».

## Other

«Cash & accounts» for balances held for completeness, and «Anything else» for a general item with your own details.

Something else you collect a lot of — model trains, quilts, rare plants? Make your own kind with its own fields: [[custom-types]].`,
  },
  {
    id: "metals",
    section: "items",
    title: "Precious metals",
    summary: "Bullion and bullion coins, valued from the spot price by weight and purity.",
    keywords: "gold silver platinum palladium bullion coin bar round spot weight purity gross fine troy ounce premium melt eagle maple",
    body: `Choose «Precious metal» when adding. The value follows the metal's spot price, so the details that matter most are weight and purity.

1. Pick a «Product» if yours is listed (American Gold Eagle, Silver Maple Leaf, 1 oz bar…). It fills in weight and purity — the figures most often entered wrong.
2. Otherwise choose the «Metal», then «Weight of one item» and its unit (troy ounces, grams, kilograms…).
3. Say whether that weight is «Gross — whole item, alloy included» or «Fine — metal content only».
4. Enter «Purity» as a fraction: .999, .9167, .900.
5. Optionally a «Premium over melt» — what a dealer pays above metal value. Blank values it at melt.
6. Leave «Value from the spot price» on to have it revalued whenever prices update.

The form shows the fine metal content and its value as you type.

> An American Gold Eagle weighs 1.0909 oz gross at .9167 — about 1 oz of gold. Getting gross and fine mixed up misprices it by about 8%.

Spot prices come from {{markets|Markets}}: type them yourself or connect a free feed. See [[spot-prices]] and [[market-pricing]].

To value metal you have not catalogued, use the [[melt-calculator]].`,
  },
  {
    id: "crypto",
    section: "items",
    title: "Cryptocurrency",
    summary: "Coins and tokens priced by CoinGecko coin ID, with optional watch-only balances.",
    keywords: "crypto bitcoin ethereum btc eth token coin id coingecko wallet exchange staked address seed phrase watch-only balance",
    body: `Choose «Cryptocurrency» when adding.

1. Pick the «Coin» — Bitcoin, Ethereum, or «Other coin or token…».
2. For other coins, enter the «Coin ID» from the coin's CoinGecko page address (for example "solana"). Symbols are ambiguous — several tokens share one — so the ID is what is priced.
3. Enter the «Quantity» held. It is exact, up to 18 decimal places.
4. Say where it is «Held in» — «My own wallet», «An exchange», or «Staked or locked» — and optionally which wallet or exchange.
5. Leave «Value from the coin price» on to follow the market.

## Watch-only address

You may add a public address so the balance can be checked later with «Check balance» on the item's page. This needs «Watch-only balance lookup» turned on in {{settings|Settings}} → «Privacy», because checking sends the address to a public block explorer.

! Never enter a seed phrase or private key. Anything that looks like one is refused before it can be stored.

Prices come from {{markets|Markets}} → «Cryptocurrency». See [[crypto-prices]].`,
  },
  {
    id: "graded",
    section: "items",
    title: "Graded collectibles and slab labels",
    summary: "Grader, grade and certificate number — and reading a slab's barcode from a photo.",
    keywords: "graded slab psa cgc bgs sgc ngc pcgs grade certificate cert number barcode scan label raw ungraded",
    body: `Comics, cards, collector coins and similar kinds have a **Grader** (or «Ungraded» for raw items), a **Grade**, and a **Cert number**.

## Reading a slab label

Choose «Scan slab label…» on the form and pick a photo of the slab's label. The barcode is read from the photo and the certificate number filled in, with the likely grader when it can tell.

> Reading the label tells you which certificate to look up — it does not verify the item. Check the certificate on the grader's site.

Certificate and serial numbers are searchable, and are listed first in insurance reports.`,
  },
  {
    id: "vehicles-property",
    section: "items",
    title: "Vehicles, property and land",
    summary: "VINs, registrations, parcels — and why only your share of the value is recorded.",
    keywords: "car truck motorcycle rv vehicle boat vin registration hull house home building property land parcel acreage share mortgage loan net worth",
    body: `«Vehicle», «Boat», «Home or building» and «Land» ask for the identifiers an insurer or buyer needs: VIN and registration, hull ID, parcel number, acreage.

For property, enter the value of **your share**. Mortgages and loans are not recorded, so totals show what you own, not your net worth.

Identifiers such as VINs, hull IDs, registrations and parcel numbers are never copied when you [[duplicate]] an item.`,
  },
  {
    id: "custom-types",
    section: "items",
    title: "Your own item types",
    summary: "Make a kind with its own fields for things the app has no form for.",
    keywords: "custom type item types fields required number date yes no list template own kind",
    body: `For things the app has no form for — model trains, quilts, rare plants — make your own kind.

1. Open {{settings|Settings}} → «Item types» → «New type…».
2. Give it a «Name», a «Category» (which decides where it counts on the overview), and an optional «Description» shown under its name when adding.
3. «Add a field» for each detail. Each field has a name and a kind: «Text», «Number», «Date», «Yes / no», or «One of a list» (enter the «Choices, separated by commas»). Tick «Required» for fields every item must have.
4. Order fields with «Move up» and «Move down», then «Create type».

The new kind appears in the add picker under «Your types». Fields are checked whenever an item is saved or imported.

Editing a type later keeps values already entered, even if you rename a field. Only a type no item uses — including items in the trash — can be deleted.`,
  },
  {
    id: "edit-item",
    section: "items",
    title: "Editing an item",
    summary: "Change details, status, notes and tags; every edit is kept in its history.",
    keywords: "edit change update details modify save changes",
    body: `Open the item and choose «Edit». Change anything and choose «Save changes».

- The **quantity** is not edited here — use «Record change» so the history shows when it changed. See [[record-change]].
- «Status» switches between «Held», «Lost» and «Retired», with the date it happened. Selling is recorded with «Record change». See [[status]].
- Every save keeps the version before it, so any edit can be undone. See [[edit-history]].

## Changing what kind of item it is

Choose «Change type…» in the form and pick the right kind. Details both kinds use carry over to the new form. Details the new kind does not use are listed separately: switch «Keep these as other details» on to keep them with the record, or off to remove them when you save. Nothing changes until you choose «Save changes».`,
  },
  {
    id: "record-change",
    section: "items",
    title: "Buying more, selling, and fixing counts",
    summary: "Record change: bought more, sold some, sold all, or fix a wrong count.",
    keywords: "record change quantity bought more buy add sell sold some sold all dispose fix count correct sale price cost partial",
    body: `Quantities change through «Record change» on the item's page, so the history shows when and why.

- «Bought more» — adds to the holding. Give the «Price paid (total)» for what you added and it is added to the cost.
- «Sold some» — reduces the holding, with an optional «Sale price (total)». Its cost is reduced in proportion.
- «Sold all» — records everything held on that date as sold. The item keeps its history and stops counting toward today's total; totals before that date still include it.
- «Fix count» — sets the quantity on a date without recording a trade, for when the number was simply wrong.

Each change has a date, so past totals and the value chart stay right.

## When some was added without a price

If you buy more without entering a price, the recorded cost no longer covers the whole holding. The item is marked «Partial cost» and no gain is shown, rather than a misleading one. Edit the item and enter what everything held cost in total to complete it — or tick that the units added without a price cost nothing extra (a gift, or already included). See [[gain]].`,
  },
  {
    id: "status",
    section: "items",
    title: "Held, sold, lost and retired",
    summary: "What each status means for totals, reports and claims.",
    keywords: "status held sold lost stolen retired recovered dispose",
    body: `- **Held** — in your possession and counted in today's totals.
- **Sold** — recorded with «Record change» → «Sold all». It leaves today's totals but stays in the catalog with its history; past totals still include it.
- **Lost** — lost, stolen or destroyed. Set it with «Edit» → «Status», with the date. It leaves today's totals, and can be included in an insurance claim with its value from before the loss. See [[claim]].
- **Retired** — no longer counted (worn out, given away) but kept for the record.

A lost or retired item can be set back to held — the history shows «Recovered».

In {{holdings|Holdings}}, the status menu shows «Held», «Sold», «Lost & retired» or «Everything».`,
  },
  {
    id: "duplicate",
    section: "items",
    title: "Duplicating an item",
    summary: "Start a similar item from an existing one.",
    keywords: "duplicate copy clone similar",
    body: `On an item's page, choose «More» → «Duplicate». A copy opens with the same kind and details, ready to edit.

The copy leaves out what must be unique or belongs to the original: serial and certificate numbers, VINs and similar identifiers, its value, its history, and its photos and documents. Edit what differs and save.

> Adding a run of similar things? «Add and start another» on the add form is quicker. See [[add-item]].`,
  },
  {
    id: "split",
    section: "items",
    title: "Splitting off part of a holding",
    summary: "Make part of a holding an item of its own, without recording a sale.",
    keywords: "split separate part holding divide move some new item",
    body: `To sell, insure or store part of a holding separately — two coins out of ten, a case of wine out of a cellar — split it off.

1. On the item's page choose «More» → «Split off part…».
2. Enter «How many» and the «Name of the new item».
3. Choose «Split off».

The part becomes an item of its own. Cost and value are divided in proportion; nothing is recorded as sold, and every total stays the same. Market-priced holdings are revalued at their new quantities. Both items show the split in their history.`,
  },
  {
    id: "trash",
    section: "items",
    title: "Deleting and restoring items",
    summary: "Deleted items wait in the trash for 30 days before they are removed for good.",
    keywords: "delete remove trash restore undo bin purge permanently empty",
    body: `On an item's page choose «More» → «Move to trash…». To trash several, select them in Holdings and choose «Trash…» (see [[selection]]).

A trashed item leaves your catalog, totals, reports and exports, but its history, photos and documents are kept. A toast offers «Undo» straight away.

## Restoring

Open {{settings|Settings}} → «Trash» and choose «Restore» beside the item. Items stay there for 30 days, then are deleted for good.

## Deleting for good

«Delete for good» beside an item, or «Empty trash», removes it from the vault immediately — its record, history, and any photos or documents nothing else uses. This cannot be undone, though backups made before then still contain it.

> If you sold the item, use «Record change» → «Sold all» instead of deleting it. That keeps its past value in your charts.`,
  },
  {
    id: "edit-history",
    section: "items",
    title: "Edit history and undo",
    summary: "Every version of an item is kept, so any edit can be undone.",
    keywords: "history undo revert versions previous restore version audit changes edits",
    body: `Each time you save changes to an item, the version before is kept. On the item's page choose «More» → «Edit history…».

The list shows each earlier version, newest first, and what differs from now. Choose «Restore this version» to bring one back. Restoring is itself an edit, so it can be undone the same way.

Other histories are kept too:

- «Holding history» on the item's page — purchases, sales, splits and count fixes.
- «Value history» — every value recorded, including voided ones. See [[void-value]].
- «Where it is» — handoffs and returns. See [[custody]].
- «Care & service». See [[care]].`,
  },
  {
    id: "asset-page",
    section: "items",
    title: "An item's page",
    summary: "Everything about one item, and everything you can do with it.",
    keywords: "asset page detail item view card more menu",
    body: `Open any item from Holdings or the Overview. Its page shows:

- **Photos** — the cover photo and a strip of the rest. See [[photos]].
- **Current value** — with where it came from and how old it is, a chart of its value over time, and «Value history». See [[values-explained]].
- **Details** — everything recorded about it, identifiers first.
- **Holding history** — when it was bought, added to, sold or split.
- **Documents** — receipts, appraisals, certificates. See [[documents]].
- **Where it is** — loans, repairs and consignments. See [[custody]].
- **Care & service** — services, repairs and warranties. See [[care]].
- Its tags, the sets it is in, and when it was last seen in an inventory check.

## Buttons

- «Update value» — record what it is worth. See [[update-value]].
- «Edit» — change its details. See [[edit-item]].
- «Record change» — bought more, sold, or fix the count. See [[record-change]].
- «More» — «Add photos…», «Edit tags…», «Duplicate», «Split off part…», «Add to a set…», «Print label…», «Edit history…» and «Move to trash…».`,
  },

  // ------------------------------------------------------------ values
  {
    id: "values-explained",
    section: "values",
    title: "How values work",
    summary: "Where each figure comes from, what it measures, and why unknown is never zero.",
    keywords: "value valuation worth price current market manual appraisal basis source provenance unvalued unknown zero as of",
    body: `Each item's **current value** is the latest value recorded for it, for the whole holding.

Every value says:

- **Where it came from** — «Entered by hand», «Market feed», or «Appraisal». Reports show this too, so an assessor can tell a market price from an estimate.
- **What it measures** — «Estimated resale» (what it would sell for), «Replacement cost», «Insured value», or «Melt value».
- **When** — the date it was true as of. A badge shows how old it is.

## Unvalued items

Items without a value are counted on the overview as needing one and listed separately — never treated as zero, which would quietly understate the total.

## Market-priced items

Metals and crypto can follow the market: their value is recalculated whenever prices update. See [[market-pricing]].

## Old values

Prices move; a figure more than a year old is flagged as stale in Catalog health. Set a reminder to revalue: [[review-reminders]].`,
  },
  {
    id: "update-value",
    section: "values",
    title: "Recording a value",
    summary: "Update value: amount, date, what it measures, and the source.",
    keywords: "update value set value price record valuation worth appraisal estimate as of date basis",
    body: `On an item's page choose «Update value» (or «Set value» from the overview).

1. «Value of the whole holding» — for everything held, not one unit.
2. «As of» — the date it was true. Use an earlier date to record a past appraisal; the chart and past totals use it.
3. «What it measures» — «Estimated resale», «Replacement cost», «Insured value» or «Melt value».
4. «Source» — «My own estimate or research» or «A professional appraisal».
5. Optionally a «Note» and «Evidence» — see [[evidence]].
6. Choose «Record».

Your own research counts — a value you looked up is better than none.

> If the item follows the market, entering a value by hand switches it to manual so a later price refresh cannot overwrite your figure. Switch back on its page. See [[market-pricing]].

To enter values for many items at once, see [[bulk-values]].`,
  },
  {
    id: "evidence",
    section: "values",
    title: "Evidence for a value",
    summary: "Comparable sales, a range, your confidence, and a supporting document.",
    keywords: "evidence comparables comps sold auction asking range low high confidence appraisal document justify",
    body: `Open «Evidence (optional)» when recording a value to say why it is what it is:

- «Add a comparable» — a similar item's price, whether it «Sold», was an «Auction result», or is «Asking», with a date and where you saw it.
- «Range» — a low and high estimate. A value outside its own range is refused.
- «Confidence» — «Low — a rough guess», «Medium», or «High — solid comparables or an appraisal».
- «Appraisal or listing on file» — pick a document already attached to the item.

Evidence appears under the value in «Value history», and in insurance reports and claims (unless you turn off «Show how values were reached»). It is what makes your own figure credible to an assessor. See [[insurance-report]].`,
  },
  {
    id: "market-pricing",
    section: "values",
    title: "Following the market",
    summary: "Metals and crypto can be revalued automatically from current prices.",
    keywords: "market price follow automatic spot coin price manual switch refresh revalue",
    body: `Metal and crypto holdings can **follow the market**: whenever spot or coin prices change, their value is recalculated from the quantity held.

- Turn it on when adding («Value from the spot price» / «Value from the coin price»), or on the item's page with «Follow the market price».
- Typing a value by hand switches the item to manual, so a later refresh cannot overwrite your figure. The page says «Now valued by hand only.»
- Switch back any time with «Follow the market price».

Prices come from {{markets|Markets}}, typed by you or fetched from a feed. See [[spot-prices]] and [[crypto-prices]].`,
  },
  {
    id: "void-value",
    section: "values",
    title: "Correcting a mistaken value",
    summary: "Void a mistyped value: it stays in the history but counts for nothing.",
    keywords: "void wrong value mistake typo correct delete valuation",
    body: `Mistyped a value? Do not record a new one on top — void it.

1. On the item's page open «Value history».
2. Choose «Void this value» beside the wrong one.
3. Give a «Reason» — typo, wrong item, duplicate entry — and choose «Void value».

The value stops counting toward the current value and every past total. It stays in the history, marked as voided with your reason, so the correction is visible.`,
  },
  {
    id: "bulk-values",
    section: "values",
    title: "Updating many values at once",
    summary: "Type values for a list of items in one go.",
    keywords: "bulk values many update values mass edit prices quickly",
    body: `1. In {{holdings|Holdings}}, filter the list to what you want to value — a category, a tag, or «Needs attention» → «No value yet».
2. Choose «More actions» → «Update many values…».
3. Type the value of each whole holding in «New value». Blank rows are left alone. «Enter» or the down arrow moves to the next row.
4. Set «As of» if the values are not for today.
5. Choose «Save values».

Rows with a problem are marked and stay for you to fix; the rest are saved. Market-tracked items you type a value for switch to manual.

> Leaving with unsaved values asks first. Sorting or filtering keeps what you have typed.

For hundreds of items, a spreadsheet may be quicker: [[csv-round-trip]].`,
  },
  {
    id: "review-reminders",
    section: "values",
    title: "Revaluation reminders",
    summary: "Get reminded when an item's value is due for a fresh look.",
    keywords: "reminder review revalue due schedule stale every month year",
    body: `Values go stale. Set «Remind me to revalue» on an item — «Every month», «Every 3 months», «Every 6 months», «Every year» or a custom number of days — in its form under «Notes & tags».

To set it for many items, select them in Holdings and choose «Reminder…». See [[selection]].

Items due appear on the {{overview|Overview}} under «Due for review» and «Needs attention», and in Holdings under «Needs attention» → «Due for review». Recording a new value restarts the clock.`,
  },
  {
    id: "currencies",
    section: "values",
    title: "Currencies and exchange rates",
    summary: "Your base currency, items valued in other currencies, and the rates that convert them.",
    keywords: "currency base usd eur gbp exchange rate convert foreign fx conversion",
    body: `## Base currency

Set in {{settings|Settings}} → «Currency» → «Base currency». New values are entered in it, totals are shown in it, and price feeds are asked for it.

## Items in other currencies

Any value can be in another currency — a watch bought in euros, a flat valued in pounds. Each keeps its own currency; nothing is ever rewritten.

To count them in totals, record an exchange rate in {{markets|Markets}} → «Exchange rates» → «Add rate»: the «From currency», «To currency», «Rate» and «As of» date. Either direction works.

Totals convert at the latest rate on or before each total's date, and say which rates were used. Without a rate, those items are listed but **left out** of totals, with a note saying so — never converted at a guess.

Earlier rates are kept («Earlier rates») so past totals stay right. Delete a mistaken rate with its trash button.`,
  },
  {
    id: "gain",
    section: "values",
    title: "Cost and gain",
    summary: "Why gain sometimes is not shown, and how to fix it.",
    keywords: "gain loss profit cost paid partial cost incomplete return",
    body: `Gain is current value minus what the whole holding cost. It is shown only when both are known and in the same currency.

No gain is shown when:

- No price paid was recorded — add one with «Edit».
- Some of the holding was added without a price («Partial cost»). Enter what everything held cost in total. See [[record-change]].
- The value and cost are in different currencies.

The overview's gain covers only items whose whole cost is known, and says over how many.

> The value chart shows what the collection was worth, not your return: a step up on a purchase date is buying, not gain. See [[value-chart]].`,
  },

  // ------------------------------------------------------------ organize
  {
    id: "holdings-list",
    section: "organize",
    title: "The holdings list",
    summary: "The whole catalog: sort, filter, switch layout, and open any item.",
    keywords: "holdings list catalog table grid sort columns category status layout show more",
    body: `{{holdings|Holdings}} lists everything in the catalog with its quantity, what you paid, its value and gain.

- **Category chips** across the top narrow to metals, collectibles, valuables and so on, with a count of each.
- The **status** menu shows «Held», «Sold», «Lost & retired» or «Everything».
- **Sort** by «Recently changed», «Name», «Value, high to low», «Gain, high to low» or «Date acquired».
- «List» or «Grid» switches between a table and photo cards.

The summary line shows how many items match and their total, plus anything without a value or in a currency with no exchange rate.

Large catalogs show the first 200 and load more as you scroll; «Show more» does the same from the keyboard.

Click a row to open the item. With the keyboard, use the arrow keys to move between rows and «Enter» to open.

See also [[search]], [[filters]], [[saved-views]] and [[selection]].`,
  },
  {
    id: "search",
    section: "organize",
    title: "Searching",
    summary: "Find items by name, notes, serials, tags, locations or documents.",
    keywords: "search find lookup query serial cert number notes tag document",
    body: `Type in the search box at the top of {{holdings|Holdings}}. Press «/» from anywhere to jump to it.

Search looks in:

- Names and notes.
- Every detail — serial and certificate numbers, brands, models, VINs.
- Storage locations and tags.
- Document titles and notes — "receipt", "Bob's appraisal".

Results are ordered by relevance and respect the filters you have set. Press «Escape» to clear the search.`,
  },
  {
    id: "filters",
    section: "organize",
    title: "Filters and “Needs attention”",
    summary: "Narrow the list by tag, location, or what is missing.",
    keywords: "filter tag location needs attention missing no value stale no photo no documents insured below away due review",
    body: `Below the search box in {{holdings|Holdings}}:

- «Any tag» — items with one tag.
- «Any location» — items kept at a place, including places inside it ("Safe" includes "Safe / Top shelf").
- «Any condition» → «Needs attention» — items missing something:
  «No value yet», «Value over a year old», «Cost incomplete», «No insured value», «Insured below value», «No photo», «No documents», «Due for review», «Away from home».

The same rules drive the overview's [[catalog-health]] card, so a count there and the list it opens always agree.

Filters combine with search, category and status. Save a combination you use often as a view: [[saved-views]].`,
  },
  {
    id: "saved-views",
    section: "organize",
    title: "Saved views",
    summary: "Save a search, filters and sort to come back to.",
    keywords: "saved views save filter preset bookmark",
    body: `Set up Holdings the way you want — search, filters, category, status and sort — then choose «Views» → «Save this view…» and name it.

Pick it from «Views» later to return to exactly that list. Remove one with «Views» → «Delete a view…».

Saved views are kept inside the encrypted vault, so they travel with it.`,
  },
  {
    id: "tags",
    section: "organize",
    title: "Tags",
    summary: "Labels you choose — for sale, insurance rider, Grandma's — searchable and filterable.",
    keywords: "tags labels tag rename merge remove",
    body: `Tags are labels you choose: anything you will want to find items by.

- **One item** — «More» → «Edit tags…», or the tags in its form. «Enter» or a comma adds a tag.
- **Many items** — select them in Holdings, then «Add tags…» or «Remove tag…». See [[selection]].
- **Rename or merge** — «More actions» → «Tags & locations…». Renaming onto an existing tag merges the two.

Filter by a tag with «Any tag» in Holdings; tags are searchable too.`,
  },
  {
    id: "locations",
    section: "organize",
    title: "Storage locations",
    summary: "Where things are kept, with places inside places.",
    keywords: "location storage where kept safe shelf deposit box room move rename nested",
    body: `Each item can have a «Storage location». Use « / » for a place inside another: «Safe / Top shelf», or "Bank / Deposit box 114".

- Filtering by a place includes everything inside it.
- **Move many items** — select them in Holdings and choose «Move to…». Each item's previous location stays in its edit history.
- **Rename or move a place** — «More actions» → «Tags & locations…». Moving a location moves everything at it, including places inside it.

Locations are left out of insurance reports unless you include them — a list of where valuables are kept is exactly what should not leak.

To check a place against the catalog, see [[inventory]].`,
  },
  {
    id: "selection",
    section: "organize",
    title: "Selecting many items",
    summary: "Select items in Holdings and change them all at once.",
    keywords: "select multiple bulk many batch checkbox select all tag move trash reminder claim labels set",
    body: `1. In {{holdings|Holdings}}, choose «Select».
2. Tick items, or click their rows. «Select all» takes every item that matches the current search and filters — including ones not yet shown.
3. Choose an action from the bar:

- «Add tags…» / «Remove tag…»
- «Move to…» — a new storage location for all of them.
- «Reminder…» — a revaluation reminder.
- «Start a claim…» — an insurance claim for exactly these. See [[claim]].
- «Print labels…» — QR labels. See [[labels]].
- «Add to set…» — see [[sets]].
- «Divide a purchase…» — split one price among them. See [[divide-purchase]].
- «Trash…» — move them to the trash.

Choose «Select» again to leave selection.`,
  },
  {
    id: "sets",
    section: "organize",
    title: "Sets and collections",
    summary: "Group items that belong together and see how complete the set is.",
    keywords: "sets collection group series checklist complete target kit",
    body: `A set groups items that belong together — a coin series, a card checklist, a tool kit — and shows how complete it is ("12 of 20") and what it is worth together.

- **Create** — {{holdings|Holdings}} → «More actions» → «Sets…» → «New set». Give it a name and, optionally, «How many make it complete».
- **Add items** — select them in Holdings and choose «Add to set…», or use «More» → «Add to a set…» on an item's page.
- **Open a set** — from «Sets…», to see its members, remove one, edit or delete it.

An item can be in several sets, and its page lists them. A set has no value of its own — its value is its members' — so nothing is counted twice. Deleting a set leaves its items in the catalog.`,
  },
  {
    id: "divide-purchase",
    section: "organize",
    title: "Dividing a purchase among items",
    summary: "Split one price paid for a lot across the items it bought.",
    keywords: "divide purchase lot allocate cost split price bundle bought together",
    body: `Bought several things for one price — a lot at auction, a collection? Catalog each item, then share out the price.

1. Select the items in {{holdings|Holdings}} (or open their set).
2. Choose «Divide a purchase…».
3. Enter the «Price paid for all of them» and its currency.
4. Choose «Divide» — «Equally», or «In proportion to their current values».

Each item's cost becomes its share; the shares add up to the price exactly, to the cent. This replaces each item's recorded cost, and the earlier figures stay in their edit history.`,
  },
  {
    id: "wishlist",
    section: "organize",
    title: "The wishlist",
    summary: "Things you want — never counted as owned.",
    keywords: "wishlist want wish list buy later target price priority got bought",
    body: `{{wishlist|Wishlist}} keeps what you are looking for, apart from what you own: wishes never count in totals or reports.

- «Add a wish» — «What», the «Type», «How many», «The most you would pay», a «Priority» and notes (condition, variant, where to look).
- If something in your catalog looks like a match, the wish says «You may already own this».
- Found it? Choose «I bought it»: the add form opens filled in from the wish, and once added the wish moves to «Got», linked to the new item.`,
  },

  // ------------------------------------------------------------ files
  {
    id: "photos",
    section: "files",
    title: "Photos",
    summary: "Add photos of an item; they are encrypted before they touch the disk.",
    keywords: "photos pictures images camera cover add remove jpeg png webp gallery",
    body: `On an item's page choose «Add photos» (or «More» → «Add photos…») and pick one or more JPEG, PNG or WebP files. They are encrypted before they are written to disk; the originals you picked are not changed or removed.

- The first photo is the **cover**, shown in lists and reports. Choose «Make cover» on another to change it.
- Click the main photo to view it full size; use the arrow keys to move between photos.
- Adding a photo already in the vault reuses the stored copy rather than storing it twice.
- «Remove photo» detaches it; if no other item uses it, the encrypted file is deleted.

Receipts and certificates belong under Documents: [[documents]].`,
  },
  {
    id: "documents",
    section: "files",
    title: "Documents",
    summary: "Receipts, appraisals, certificates, warranties and manuals, kept with the item.",
    keywords: "documents receipt invoice appraisal certificate warranty manual pdf paperwork attach file",
    body: `Proof of ownership and value belongs with the item. On its page, under «Documents», choose «Add document…».

1. Pick a PDF or a photo of the paperwork.
2. Say «What it is» — «Receipt», «Appraisal», «Certificate», «Warranty», «Manual» or «Document».
3. Optionally a «Title» (blank uses the file name), the «Date on the document», and a «Note».

Documents are encrypted like everything else, and their titles and notes are searchable.

Beside each document:

- **View** — open it in the app. See [[pdf-viewer]].
- **Save a copy…** — export it. See [[save-copy]].
- **Edit details** and **Remove**.

Catalog health counts items with «No documents», and insurance reports can list documents on file by title.`,
  },
  {
    id: "pdf-viewer",
    section: "files",
    title: "Viewing PDFs",
    summary: "Open PDF documents inside the app — decrypted in memory only.",
    keywords: "pdf viewer open view document zoom page next previous password protected",
    body: `Click a document's title, or its eye button, to open it. PDFs open in the built-in viewer; photos open full size.

- The up and down arrows (or «Page Up» / «Page Down») move a page at a time; the page number shows where you are.
- «−» and «+» zoom; «Fit width» fits the page to the window. «Ctrl» + «+» / «−» work too.
- «Save a copy…» exports it. See [[save-copy]].

The document is decrypted into memory only, never to disk, and cleared when the viewer closes. Nothing in a PDF runs — no scripts or forms.

> A password-protected PDF cannot be shown in the app. Save a copy and open it in a PDF reader to enter its password.`,
  },
  {
    id: "save-copy",
    section: "files",
    title: "Saving an unencrypted copy",
    summary: "Export a photo or document to send or open elsewhere.",
    keywords: "save copy export download decrypt file send unencrypted",
    body: `To send a receipt or open a document in another app, choose «Save a copy…» beside it (or in the PDF viewer), confirm, and choose where to save.

! The copy is an ordinary file, readable by anything that can open the folder you choose — outside the vault's protection. Delete it when you are done. The original stays encrypted in the vault.

For a claim, «Save receipts & photos…» exports all of a claim's files at once. See [[claim]].`,
  },

  // ------------------------------------------------------------ care
  {
    id: "care",
    section: "care",
    title: "Care, service and warranties",
    summary: "Services, repairs, inspections and warranties, with when the next is due.",
    keywords: "care service repair inspection cleaning battery warranty maintenance due next",
    body: `Keep the history a buyer or insurer asks for: on an item's page, under «Care & service», choose «Add care or service».

- «What» — «Service», «Repair», «Inspection», «Cleaning», «Appraisal», «Battery», «Warranty» or «Other care».
- «Done on» — leave blank for something only due, like a warranty's end.
- «By» — who did it — and «Cost», which is not added to what it cost to buy.
- «Next due» — shown on the overview under «Coming up» as it approaches.
- «Invoice or document» — link a document already attached.

The {{overview|Overview}}'s «Coming up» card lists care, warranties and returns due within 60 days.`,
  },
  {
    id: "custody",
    section: "care",
    title: "Loans, repairs and consignments",
    summary: "Record who has something, and when it is due back.",
    keywords: "custody loan lent borrowed consigned consignment repair shop storage shipped away hand over back home due back",
    body: `When something leaves your hands, record it: on its page, under «Where it is», choose «Hand over…».

- «What» — «Lent», «Consigned», «At repair», «In outside storage» or «Shipped».
- «To» — who has it, and an optional «Contact» (kept in the vault; never in exports or reports).
- «Due back», a «Reference» (consignment, ticket or tracking number), a linked document, and a note.

When it returns, choose «Mark as back…» and the date it came back.

Items away show where they are in Holdings, and «Needs attention» → «Away from home» lists them all. Anything overdue is flagged on the {{overview|Overview}} under «Coming up». An inventory check does not expect items that are away.`,
  },
  {
    id: "inventory",
    section: "care",
    title: "Inventory checks",
    summary: "Walk a room, safe or storage unit and confirm each thing is there.",
    keywords: "inventory check audit count stocktake present missing scan label last seen verify room safe",
    body: `1. In {{holdings|Holdings}} choose «More actions» → «Inventory checks…».
2. Choose «Where» — a place, which includes the places inside it — or «Everything held». Give the check a «Name» and choose «Start».
3. For each item, choose «Here» or «Missing», or type a number and choose «Count» when you counted a different quantity. Or scan its label (or type its short code) into «Scan a label» and press «Enter» to mark it here.
4. Choose «Finish and review» when done. A check is saved as you go, so you can «Resume» it later.

The review lists what is **missing**, what has a **different count**, and what was **not checked**. Nothing is changed for you — a missing item may only be misplaced:

- «Open — mark lost or note it» for a missing item.
- «Correct to …» to fix a count; it is recorded as a correction from this check.

Each item marked here gets a «Last seen» date on its page. Deleting a check removes its marks and those dates.

Labels make checks quick: [[labels]].`,
  },
  {
    id: "labels",
    section: "care",
    title: "Printing QR labels",
    summary: "Labels to stick on items or boxes, for quick inventory checks.",
    keywords: "labels qr code print sticker barcode scan short code",
    body: `- One item: «More» → «Print label…».
- Many: select them in Holdings and choose «Print labels…».

Tick «Print the item's name under the code» if you want it, then «Print».

Each label's code identifies the item only to this vault — it holds no value, location or serial number. Scan it during an [[inventory]] check, or type the short code printed under it.`,
  },

  // ------------------------------------------------------------ overview & markets
  {
    id: "overview",
    section: "overview",
    title: "The overview",
    summary: "Total value, value over time, what needs attention and what is coming up.",
    keywords: "overview dashboard home total value collection summary tiles allocation largest recent activity",
    body: `{{overview|Overview}} is the first screen after unlocking.

- **Collection value** — everything held that has a value, in your base currency, with the gain against cost where the whole cost is known.
- **Holdings** — how many items, and the cost of those valued.
- **Need a value** — items counted but not in the total. Click to list them.
- **Due for review** — items whose revaluation reminder is due.
- **Value over time** — see [[value-chart]].
- **Allocation** — how value splits across categories.
- **Largest holdings** — the items worth most.
- **Needs attention** — what has no value yet or is due for review, with «Set value» buttons.
- **Recent activity** — the latest purchases, sales, splits and count fixes.
- **Catalog health** — see [[catalog-health]].
- **Coming up** — care, warranties and returns due within 60 days.

A banner reminds you when the vault has not been backed up recently. See [[backups]].`,
  },
  {
    id: "catalog-health",
    section: "overview",
    title: "Catalog health",
    summary: "What would make your totals more complete and the catalog more useful after a loss.",
    keywords: "catalog health checks missing value photo document insurance stale underinsured",
    body: `The «Catalog health» card on the {{overview|Overview}} checks every held item:

- «Every item has a value» — unvalued items are counted as unknown, never as zero.
- «Every value is less than a year old» — prices move; an old figure is a guess.
- «No incomplete costs» — items bought more of without a price show no gain.
- «Nothing insured below its value» — a recorded insured value lower than the current value.
- «Every item has a photo».
- «Every item has a document on file» — proof of ownership and value.

Each line with something to fix has «Show», which opens Holdings filtered to exactly those items.

It also warns when most of the value is kept in one place, or when one item is a large share of the total — one event there would affect most of the catalog.`,
  },
  {
    id: "value-chart",
    section: "overview",
    title: "Reading the value-over-time chart",
    summary: "What the collection was worth on each date — value, not return.",
    keywords: "chart graph value over time history trend dashed dots table range 3m 1y 3y all",
    body: `The «Value over time» chart shows what everything held was worth on each date. Choose «3M», «1Y», «3Y» or «All» for the range.

- It shows **value, not return**. When you buy something the line steps up — that is a purchase, not a gain. Dots mark the dates a holding changed.
- The line is **dashed** where some holdings had no value on that date, so the figure is partial.
- Items valued in a currency with no exchange rate are left out, and the chart says so.

Choose «Show as table» to read the figures date by date — useful with a screen reader.

Each item's page has its own value chart.`,
  },
  {
    id: "spot-prices",
    section: "overview",
    title: "Metal spot prices",
    summary: "Type spot prices yourself, or update them from metals.dev.",
    keywords: "spot price gold silver platinum palladium metals.dev feed update set by hand freshness stale quota",
    body: `{{markets|Markets}} → «Precious metals» shows the spot price per troy ounce of gold, silver, platinum and palladium, with how fresh each is.

## Typing a price

Enter the price in the metal's box and choose «Set». It costs nothing and works offline. Every holding following that metal is revalued.

## Updating from a feed

With a free metals.dev key (see [[price-feeds]]), choose «Update from metals.dev» to fetch all four at once. The meter shows how many of the month's requests are left. You can also have metals refresh when you unlock («Refresh metals on unlock»).

Freshness is judged by when the source priced the metal, not when it was fetched — a badge turns amber when a price is old.`,
  },
  {
    id: "crypto-prices",
    section: "overview",
    title: "Crypto prices",
    summary: "Coin prices for your crypto holdings, typed or fetched from CoinGecko.",
    keywords: "crypto prices coingecko bitcoin update coin price set by hand",
    body: `{{markets|Markets}} → «Cryptocurrency» lists each coin you hold, how much, its price and how old the price is.

- Type a price in «Set by hand» and choose «Set».
- Or, with a CoinGecko key (see [[price-feeds]]), choose «Update from CoinGecko». All your coins are priced in one request, and only coins you hold are asked for.

Coins are priced by coin ID, not ticker — several tokens can share a symbol. If a coin is reported missing, check its coin ID on the item.`,
  },
  {
    id: "melt-calculator",
    section: "overview",
    title: "Melt calculator",
    summary: "Value metal you have not catalogued, at the stored spot price.",
    keywords: "melt calculator value metal scrap weight purity premium",
    body: `{{markets|Markets}} → «Melt calculator» values metal you have not catalogued — scrap jewelry, a coin you are thinking of buying.

Choose a «Product» or the «Metal», enter the «Quantity», «Weight each» and its unit, whether the weight is «Gross» or «Fine», the «Purity», and an optional «Premium %». Choose «Calculate».

It shows the fine metal, the **melt value** and the **market value** with premium apart — a coin's premium can dwarf its metal — and which spot price it used.`,
  },
  {
    id: "price-feeds",
    section: "overview",
    title: "Price feed keys",
    summary: "Optional free keys for one-click metal and crypto prices.",
    keywords: "api key metals.dev coingecko price feed keyring refresh automatic",
    body: `Everything works with prices typed by hand. A free key adds one-click updates:

- **metals.dev** — gold, silver, platinum and palladium spot. The free plan allows 100 requests a month; one request updates all four.
- **CoinGecko** — coin prices. The free Demo plan allows 10,000 calls a month; all your coins are priced in one request.

Paste a key in {{settings|Settings}} → «Price feeds» and choose «Save». Keys are kept in your system keyring, never in the vault or a file.

«Refresh metals on unlock» fetches metals at most every 30 hours, from a small share of the monthly allowance kept for automatic use.

> Asking a provider for prices tells it which metals or coins you hold — nothing else. See [[network]].`,
  },

  // ------------------------------------------------------------ reports
  {
    id: "insurance-report",
    section: "reports",
    title: "Insurance inventory report",
    summary: "Every held item with photos, details, cost and value — to print or save as PDF.",
    keywords: "insurance report inventory print pdf insurer document photos schedule rider",
    body: `{{reports|Reports}} → «Insurance inventory» builds a document of everything held: photos, identifying details, cost, value — and where each value came from, so an assessor can tell a market price from an estimate.

Choose what to include:

- «Include photos» — up to four per item.
- «List documents on file» — receipts, appraisals and certificates by title and date, not their contents.
- «Show how values were reached» — comparables, range and confidence. See [[evidence]].
- «Include storage locations» — off by default; usually unnecessary, and a list of where valuables are kept should not leak.
- «Include notes».
- «Include items marked lost» — each with the date lost and its value from before.

Choose «Preview report», then «Print or save as PDF».

! The printed report and any PDF you save are not encrypted. Treat the file as you would the items themselves.`,
  },
  {
    id: "claim",
    section: "reports",
    title: "Preparing an insurance claim",
    summary: "Just the items lost, valued as they were before the loss, with their paperwork.",
    keywords: "claim insurance lost stolen damaged theft fire flood date of loss policy insurer itemized",
    body: `For items lost, stolen or damaged, a claim lists only those items — nothing else in your catalog is disclosed — valued as they were before the loss.

1. Open {{reports|Reports}} → «Prepare a claim…», or select the items in Holdings and choose «Start a claim…».
2. Tick the items the claim is for («Find items…» narrows the list) and choose «Preview the claim».
3. Fill in «The claim»: «Date of loss», «Claim number», «Insurer», «Policy number», «Your name» and «What happened». These go on the document only; they are not saved.
4. Choose what to include — photos, receipts and appraisals, how values were reached, every value on record, storage locations.
5. Review the preview, then «Print or save as PDF».

Values are those recorded **on or before the date of loss**. If an item has none, the preview says so — record a value with an earlier date if you have a receipt or appraisal.

## Sending the paperwork

«Save receipts & photos…» decrypts the chosen items' receipts, appraisals and certificates — and photos, if included — into a new folder to send with the claim.

! The claim document and saved files are not encrypted. Delete them when the claim is settled.

> Mark the items as lost (see [[status]]) so they leave your totals.`,
  },
  {
    id: "spreadsheet-import",
    section: "reports",
    title: "Importing your own spreadsheet",
    summary: "Bring in an inventory you keep in Excel, Numbers or Google Sheets — any columns, checked row by row.",
    keywords: "import spreadsheet csv excel numbers google sheets columns mapping template migrate bring in",
    body: `Already keep a list? {{reports|Reports}} → «Import your own spreadsheet» → «Import a spreadsheet…».

1. Save it as CSV from Excel, Numbers or Google Sheets. Comma, semicolon and tab separated files all work. (Starting fresh? «Save a template…» gives you a file to fill in.)
2. Choose the file. For each column, say «What … holds» — name, quantity, price paid, value, date bought, location, tags, a detail like serial number — or «Leave out».
3. Set «Type for rows without one», the «Currency of amounts», the «Decimal mark», and the order «Dates are written».
4. Check the preview: every row is shown as it will be stored, with problems and likely duplicates of items already in your catalog pointed out.
5. Fix problems in the file, or «Leave out the … with problems», then «Import».

Prices are read as written — "$1,299.50" or "1.299,50". The next file with the same columns is read the same way automatically.

Nothing is added until you choose «Import», and then every row is added together or none are.`,
  },
  {
    id: "csv-round-trip",
    section: "reports",
    title: "Editing in a spreadsheet (CSV round trip)",
    summary: "Export, edit hundreds of items in a spreadsheet, and import the changes.",
    keywords: "csv export import round trip spreadsheet bulk edit re-import update",
    body: `For a few hundred items, export → edit in a spreadsheet → import beats any form.

1. {{reports|Reports}} → «Spreadsheet round trip» → «Export CSV…».
2. Edit the file in a spreadsheet.
3. «Import CSV…» (or Holdings → «More actions» → «Re-import an export…»), check the preview, and confirm.

Rules:

- Each row keeps its asset ID, so importing updates rather than duplicating. Rows without an ID are added as new items.
- A blank cell leaves the stored value alone; a single dash (-) clears it.
- Amounts are in minor units (cents): 1299.50 is written 129950.
- A changed value becomes a dated valuation; a changed quantity becomes a correction in the history.
- An old export is refused if the catalog has changed since, so it cannot overwrite newer edits.

! CSV is plain text with no photos or history. It is not encrypted and not a backup — see [[backups]].`,
  },
  {
    id: "emergency-sheet",
    section: "reports",
    title: "Emergency access sheet",
    summary: "Instructions for someone you trust to find and open the catalog.",
    keywords: "emergency access estate executor family death incapacity instructions sheet trust",
    body: `If something happened to you, could someone you trust find and open this catalog? {{settings|Settings}} → «Emergency access» → «Print an access sheet…» prints instructions for them:

- What they need — the app, the vault or a backup, and the recovery key.
- Where things are — where backups are kept, and where the recovery key sheet is kept.
- How to open it, on this computer or another.
- Who else can help, and anything else they should know.

The sheet holds no passphrase or key — on its own it opens nothing. Keep it apart from your recovery key sheet. What you type is printed, not saved. Reprint it after moving the vault or its backups.`,
  },

  // ------------------------------------------------------------ safety
  {
    id: "backups",
    section: "safety",
    title: "Backups",
    summary: "Make a complete, still-encrypted backup of the vault — safe on an external drive.",
    keywords: "backup back up copy external drive usb synced folder reminder keep schedule",
    body: `A backup is a complete, still-encrypted copy of the vault — records, photos, documents and history. Without your passphrase or recovery key it is unreadable, so it is safe on an external drive or in a synced folder.

1. {{settings|Settings}} → «Backups» → «Choose where to put backups». An external drive or a synced folder, away from this computer, is best.
2. Choose «Back up now».
3. Verify it — see [[verify-backup]].

Options:

- «Keep in that folder» — «The newest 3», 5, 10 or 20, or «Keep every backup». Older backups of this vault are removed after each new one; nothing else in the folder is touched.
- «Remind me to back up» — after a week, two weeks, a month or three months. The reminder shows on the overview while the app is open.
- «Back up to another folder…» for a one-off copy elsewhere.

> Backups made with an old passphrase or recovery key still open with that one. After changing either, make a fresh backup.`,
  },
  {
    id: "verify-backup",
    section: "safety",
    title: "Verifying a backup",
    summary: "The only proof a backup will open on a new computer.",
    keywords: "verify backup test check restore proof integrity",
    body: `{{settings|Settings}} → «Backups» → «Verify» beside a backup (or «Verify a backup…» for one elsewhere).

The backup is restored into a scratch folder, every photo and document is decrypted, and the copy is removed. Your vault is not touched. Give the passphrase or recovery key the backup was made with.

The result is recorded: «This backup restores» with what was checked, or why it failed. «Last backup proven to restore» shows when one last passed.`,
  },
  {
    id: "restore",
    section: "safety",
    title: "Restoring a backup or moving computers",
    summary: "Open a backup on this or another computer; every file is checked first.",
    keywords: "restore move new computer migrate transfer backup recover replace pre-restore",
    body: `## On a new computer

1. Copy the backup folder over, or plug in the drive it is on.
2. Install Asset Manager and choose «Restore from a backup».
3. «Choose folder…» — the backup folder, the one containing manifest.json.
4. Give the passphrase the backup opens with — or choose «Use the recovery key instead».
5. Choose «Verify and restore».

## On this computer

{{settings|Settings}} → «Backups» → «Restore…».

The backup is opened and checked — every record and photo — before anything is replaced. If the check fails, nothing changes.

Use the passphrase or recovery key the vault had **when the backup was made** — not necessarily today's.

A vault already on the computer is set aside as "vault.pre-restore", not deleted.`,
  },
  {
    id: "change-passphrase",
    section: "safety",
    title: "Changing your passphrase",
    summary: "Change what opens the vault from now on.",
    keywords: "change passphrase password new update",
    body: `{{settings|Settings}} → «Security» → «Change passphrase». Enter the «Current passphrase», then the «New passphrase» twice (at least 12 characters).

Your photos and records are not re-encrypted — only the key that unlocks them is re-wrapped — so this takes a second. It is done in a way that cannot leave the vault unopenable if interrupted.

> Older backups still open with the passphrase they were made with. Make a fresh backup afterwards so one opens with the new passphrase.`,
  },
  {
    id: "new-recovery-key",
    section: "safety",
    title: "Issuing a new recovery key",
    summary: "Replace a recovery key that may have been seen.",
    keywords: "new recovery key rotate replace compromised seen fingerprint",
    body: `If your recovery key sheet may have been seen, replace it: {{settings|Settings}} → «Security» → «New recovery key».

The current key stops opening this vault immediately. You are shown the new key once and must confirm you have saved it, as when the vault was created ([[recovery-key]]). Destroy the old sheet.

> Backups made before now still open with the old key. Make a fresh backup, and destroy old backups you no longer trust.`,
  },
  {
    id: "forgot-passphrase",
    section: "safety",
    title: "If you forget your passphrase",
    summary: "Open the vault with the recovery key, then set a new passphrase.",
    keywords: "forgot passphrase password lost locked out cannot unlock reset recovery",
    body: `1. On the unlock screen choose «Use the recovery key instead».
2. Type the key from your recovery sheet and choose «Unlock».
3. Set a new passphrase in {{settings|Settings}} → «Security» → «Change passphrase».
4. Make a fresh backup.

A backup opens with the passphrase or recovery key the vault had when it was made — try older passphrases if you have changed it since.

! There is no password reset. If both the passphrase and the recovery key are lost, the vault cannot be opened by anyone — not support, not the app's author.`,
  },
  {
    id: "portable",
    section: "safety",
    title: "Portable mode",
    summary: "Keep the app and its catalog together in one folder — on a USB stick, say.",
    keywords: "portable usb stick thumb drive removable carry marker folder appimage standalone exe assetmanagerdata",
    body: `Normally the vault lives in your user profile. The portable builds keep it beside the app instead, so the app and your catalog can travel together.

## Getting it

Download the portable build — **AssetManager_…_portable.tar.gz** for Linux or **AssetManager_…_portable.zip** for Windows — and unpack it into a folder of your own (or onto a USB stick). It holds the app and a small marker file, **assetmanager-portable**. On first launch the app creates **AssetManagerData** beside them, and keeps the vault and your preferences there.

{{settings|Settings}} → «This vault» shows «Storage» as portable when it is on.

## Moving it

Move the app, the **assetmanager-portable** marker and the whole **AssetManagerData** folder together. If the data folder is there but the marker is missing, the app stops with a message rather than quietly opening a different (or empty) catalog.

## Good to know

- The vault is encrypted wherever it is: a lost USB stick shows nothing without your passphrase or recovery key.
- Price-feed keys are kept in each computer's own keyring, so enter them again on each computer you use.
- Backups still go where you choose — keep one off the stick, too. See [[backups]].
- Portable mode works with the Linux AppImage and the standalone Windows executable. It is not available for installed copies (.deb, .msi, macOS), whose folders may be shared, read-only or replaced by an update — nor for a copy run from Program Files.
- On Windows the portable app needs Microsoft's WebView2, which Windows 10 and 11 normally include.

! Portable mode does not move a vault you already have. To bring one over, make a backup and restore it in the portable copy — see [[restore]].`,
  },
  {
    id: "security",
    section: "safety",
    title: "How your data is protected",
    summary: "What is encrypted, what is not, and what to be careful with.",
    keywords: "security encryption encrypted privacy protection argon2 sqlcipher xchacha keyring memory",
    body: `- **The vault** — records and history are in an encrypted database; photos and documents are encrypted files with meaningless names. Without the passphrase or recovery key, a stolen copy is unreadable.
- **The passphrase** — turned into a key with Argon2id, which makes guessing slow. It is never stored.
- **While unlocked** — decrypted data exists only in the app's memory. Locking drops it. Photos are never cached to disk.
- **API keys** — kept in your system keyring, not the vault.

## Not encrypted — handle with care

- Printed reports, claims and the PDFs you save from them.
- CSV exports.
- Copies you save with «Save a copy…».
- The printed recovery key sheet — it opens the vault.

{{settings|Settings}} → «This vault» shows where the vault is, its format and encryption.`,
  },
  {
    id: "network",
    section: "safety",
    title: "What the app sends over the internet",
    summary: "Nothing, unless you turn on a price feed or a balance lookup.",
    keywords: "network internet privacy offline tracking telemetry analytics send data online",
    body: `Asset Manager works fully offline. It has no account, no analytics, no telemetry and no update check.

Only three optional features go online, each only when you use or enable it:

- **metals.dev** — asked for spot prices. It learns that someone at your IP address wants metal prices.
- **CoinGecko** — asked for the prices of the coins you hold, so it learns which coins those are.
- **Balance lookup** (off by default, {{settings|Settings}} → «Privacy») — sends a watch-only Bitcoin address to blockstream.info, which learns that someone at your IP is interested in it.

No item names, values, photos or locations are ever sent.`,
  },

  // ------------------------------------------------------------ reference
  {
    id: "theme",
    section: "reference",
    title: "Light and dark theme",
    summary: "Follow the computer's setting, or always use light or dark.",
    keywords: "theme dark mode light mode appearance colors colours night",
    body: `Open {{settings|Settings}} → «Appearance» → «Theme» and choose:

- «System» — follow your computer's light or dark setting, and change with it.
- «Light» or «Dark» — always use that, whatever the computer is set to.

The choice belongs to this computer rather than the vault, so it also applies on the unlock screen — and a vault restored on another computer uses that computer's choice.

Printed reports, claims, labels and recovery sheets are always printed dark on white.`,
  },
  {
    id: "shortcuts",
    section: "reference",
    title: "Keyboard shortcuts",
    summary: "Keys that work across the app.",
    keywords: "keyboard shortcuts keys hotkeys accessibility",
    body: `## Anywhere

- «/» — jump to search (Holdings search if the screen has none).
- «?» — open this help.
- «Escape» — close a dialog or menu.

## Lists

- Up and down arrows move between rows; «Enter» opens the item.
- In selection, «Space» ticks the row.
- In the search box, «Escape» clears it.

## Updating many values

- «Enter» or the down arrow moves to the next row; the up arrow to the previous.

## Photos and PDFs

- Left and right arrows move between photos.
- In the PDF viewer, up/down arrows or «Page Up» / «Page Down» move a page; «Ctrl» + «+» / «−» zoom.

## Inventory checks

- Scan a label (or type its code) and press «Enter» to mark it here.`,
  },
  {
    id: "troubleshooting",
    section: "reference",
    title: "Troubleshooting",
    summary: "Common surprises and what to do about them.",
    keywords: "troubleshooting problem help issue not working wrong missing error fix faq",
    body: `## An item is not in the total

It has no value yet («Need a value» on the overview), or it is valued in a currency with no exchange rate — add one in Markets. See [[currencies]].

## No gain is shown

The cost is missing, incomplete, or in a different currency. See [[gain]].

## A metal or coin's value did not change when prices updated

It was switched to manual when a value was typed by hand. Choose «Follow the market price» on its page. See [[market-pricing]].

## The chart jumped up

A purchase, not a gain — the chart shows value, not return. See [[value-chart]].

## A spreadsheet import was refused

A re-imported export is refused if the catalog changed since it was made — export again. Other problems are listed per row in the preview. See [[csv-round-trip]].

## A PDF will not open

Password-protected PDFs cannot be shown in the app — save a copy and open it in a PDF reader.

## The backup folder says “Not found”

The drive is unplugged or the folder moved. Plug it in, or «Choose another…».

## The vault locked by itself

It locks after inactivity — change the time in Settings → «Security». See [[unlocking]].

## I deleted something by mistake

Restore it from Settings → «Trash» within 30 days. See [[trash]].`,
  },
  {
    id: "glossary",
    section: "reference",
    title: "Glossary",
    summary: "Terms used in the app.",
    keywords: "glossary terms definitions meaning",
    body: `- **Vault** — the encrypted folder holding your catalog.
- **Recovery key** — a second way to open the vault, kept offline.
- **Fingerprint** — identifies which recovery key belongs to a vault; reveals nothing about it.
- **Holding** — one catalog entry, which can be several of a thing (20 coins).
- **Basis** — what a value measures: estimated resale, replacement cost, insured value, or melt value.
- **Source** — where a value came from: entered by hand, a market feed, or an appraisal.
- **Spot price** — the current market price of a metal, per troy ounce.
- **Troy ounce** — the unit metals are priced in: 31.1035 grams.
- **Gross / fine weight** — the whole item's weight, or the pure metal in it.
- **Purity** — the fraction of metal: .999 is 99.9%.
- **Melt value** — what the metal alone is worth at spot.
- **Premium** — what a dealer pays above melt.
- **Certificate (cert) number** — a grading company's number for a slabbed item.
- **Coin ID** — CoinGecko's identifier for a cryptocurrency, which is unique where symbols are not.
- **Minor units** — the smallest unit of a currency: cents for dollars.
- **Partial cost** — some of a holding was added without a price, so its cost covers only part of it.
- **Void** — mark a mistaken value so it counts for nothing, while keeping it in the history.`,
  },
];
