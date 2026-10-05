-- Schema v14: exchange rates the owner records.
--
-- Holdings valued in another currency used to be left out of every total.
-- With a rate on record, a total converts them at the latest rate on or
-- before its own date — today's figure at today's rate, March's at March's
-- — and says which rates it used. Original amounts are never rewritten:
-- a valuation in euros stays in euros. Rates are entered by hand; fetching
-- them would tell a provider which currencies the owner holds.
CREATE TABLE fx_rates (
    rate_id          TEXT    PRIMARY KEY,
    from_currency    TEXT    NOT NULL,
    to_currency      TEXT    NOT NULL,
    -- 1 unit of from_currency is worth `rate` units of to_currency.
    rate             TEXT    NOT NULL,
    asof             TEXT    NOT NULL,
    source           TEXT    NOT NULL DEFAULT 'manual',
    recorded_at      TEXT    NOT NULL,
    CHECK (from_currency <> to_currency)
) STRICT;

CREATE INDEX fx_rates_pair ON fx_rates (from_currency, to_currency, asof);
