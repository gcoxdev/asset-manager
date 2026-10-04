-- Schema v8: documents, not just photos.
--
-- Receipts, appraisals, certificates and warranties were attachable but
-- showed as an anonymous "PDF" — a receipt no one can find is not evidence.
-- Each attachment now says what it is, carries a readable title and the
-- document's own date, and can be searched by title or note. All of it
-- lives in the encrypted database, like everything else about an asset.
ALTER TABLE asset_media ADD COLUMN doc_kind TEXT NOT NULL DEFAULT 'photo'
    CHECK (doc_kind IN ('photo', 'receipt', 'appraisal', 'certificate', 'warranty', 'manual', 'other'));
ALTER TABLE asset_media ADD COLUMN title TEXT;
ALTER TABLE asset_media ADD COLUMN doc_date TEXT;
ALTER TABLE asset_media ADD COLUMN note TEXT NOT NULL DEFAULT '';

-- Anything already attached that is not an image was a document.
UPDATE asset_media SET doc_kind = 'other'
WHERE object_id IN (SELECT object_id FROM objects WHERE media_type NOT LIKE 'image/%');

-- Only a photo can be the cover photo.
UPDATE asset_media SET is_primary = 0 WHERE doc_kind <> 'photo';
