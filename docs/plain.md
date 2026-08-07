# Pedro Search Architecture Plan

## Summary

Pedro is a serverless search system for documents with user-defined schemas. It combines:

- Lexical search using Japanese-aware tokenization and BM25 ranking
- Native vector search for semantic retrieval
- Exact-match and `IN` filters over fields defined by users
- Online schema evolution, including adding and removing filterable fields
- A Lambda-first execution model without resident search servers

The initial serving architecture should use DynamoDB for lexical postings and facets, S3 Vectors for vector retrieval, and S3 as the durable source for raw documents and index rebuilds.

```text
S3
  Raw documents, exports, and rebuild input

DynamoDB
  Schema registry
  Text postings
  Dynamic facet postings
  Term and facet statistics
  Forward document index
  Search result snapshots with TTL

S3 Vectors
  Vector embeddings
  Vector metadata filters

Lambda and SQS
  Document indexing
  Schema backfills
  Search execution
```

This design favors operational simplicity and burst-friendly serverless execution. A future S3 segment-based lexical backend can replace DynamoDB if posting storage cost or common-term query cost becomes a bottleneck.

## Requirements

### Search

- Return up to 5,000 ranked lexical results internally.
- Return up to 100 vector results.
- Support Japanese text analysis with Kuromoji, Lindera, or an equivalent analyzer.
- Support BM25 ranking, field boosts, Boolean queries, and phrase queries where configured.
- Allow vector and lexical results to be combined with Reciprocal Rank Fusion or another reranker.

### Filters

Filters are exact-match keyword filters. The API must support expressions such as:

```text
documentId IN (doc1, doc2)
projectId IN (project1, project2)
hogeId IN (hoge1)
```

Multiple values for one field form a union. Constraints across fields form an intersection:

```text
(doc1 OR doc2)
AND
(project1 OR project2)
AND
hoge1
AND
textMatches
```

The initial implementation does not need arbitrary ranges, user-defined sorting, aggregations, or unbounded negation.

### Schema evolution

Users can:

- Define searchable text fields.
- Define filterable keyword fields.
- Add or remove filterable fields later.
- Change a field configuration through a new generation.

Adding a filterable field to existing documents requires an asynchronous backfill. The field becomes queryable only after the backfill reaches `ACTIVE`.

## Public schema model

Example:

```json
{
  "fields": {
    "content": {
      "type": "text",
      "searchable": true,
      "analyzer": "ja-kuromoji"
    },
    "documentId": {
      "type": "keyword",
      "filterable": true,
      "unique": true
    },
    "projectId": {
      "type": "keyword",
      "filterable": true
    }
  }
}
```

The control plane assigns a stable internal `fieldId` to every logical field name:

```text
content    -> f001
documentId -> f002
projectId  -> f003
hogeId     -> f004
```

Storage keys use `fieldId`, not the user-provided name. Renaming a field can then remain a metadata-only operation. Reusing a deleted name must create a new `fieldId`.

## Query API

Prefer a typed query AST over embedding SQL-like syntax in a free-form string.

```json
{
  "query": {
    "match": {
      "field": "content",
      "text": "building a machine learning model"
    }
  },
  "filter": {
    "and": [
      {
        "in": {
          "field": "documentId",
          "values": ["doc1", "doc2"]
        }
      },
      {
        "in": {
          "field": "projectId",
          "values": ["project1", "project2"]
        }
      },
      {
        "in": {
          "field": "hogeId",
          "values": ["hoge1"]
        }
      }
    ]
  },
  "lexicalLimit": 5000,
  "vectorLimit": 100,
  "pageSize": 100
}
```

A single-value `IN` expression may be normalized to equality internally.

## DynamoDB data model

Do not create a GSI or table for every user-defined field. Use generic index tables keyed by `indexId`, stable `fieldId`, field generation, and canonical value.

### Schema registry

```text
PK = INDEX#{indexId}
SK = FIELD#{fieldId}

name
type
searchable
filterable
analyzer
generation
status        ACTIVE | BACKFILLING | DISABLED | DEPRECATED
```

### Text postings

```text
PK = INDEX#{indexId}#FIELD#{fieldId}#GEN#{generation}
     #TERM#{token}#SHARD#{shard}
SK = IMPACT#{inverseImpact}#DOC#{documentId}

tf
documentLength
positions
revision
```

The sort key starts with an approximate term impact so high-value candidates can be read first. Exact BM25 is calculated after candidate collection.

Positions should be stored only for fields that enable phrase queries because they substantially increase storage and write amplification.

### Facet postings

```text
PK = INDEX#{indexId}#FIELD#{fieldId}#GEN#{generation}
     #VALUE#{valueHash}#SHARD#{shard}
SK = DOC#{documentId}

canonicalValue
revision
```

Values must be type-tagged and canonicalized before hashing. The original canonical value is retained for collision verification.

### Forward document index

```text
PK = INDEX#{indexId}
SK = DOC#{documentId}

revision
fieldValues
fieldLengths
sourceLocation
```

The forward index supports direct `documentId IN (...)` execution, filter verification, deletion, and targeted reindexing.

### Statistics

Maintain approximate statistics for query planning:

```text
term document frequency
facet value document count
posting shard count
maximum impact per shard or page
document count
average field length
```

Counts do not need transactional precision. They are planning and ranking inputs and can be reconciled asynchronously.

### Search result snapshots

Do not return all 5,000 documents in one response. Materialize result IDs and scores in pages:

```text
PK = SEARCH#{searchId}
SK = PAGE#{pageNumber}

documentIds
scores
expiresAt
```

A page should normally contain 50 to 100 entries. The API returns a cursor containing the search ID and next page number.

## Filter execution

### Direct document ID filter

When `documentId IN (...)` is present and the set is small, use `BatchGetItem` against the forward index and evaluate text and remaining filters over that bounded set. Do not create or read a facet posting list for a unique document ID unless it is required for another access pattern.

### Generic facet filter

For a filter such as:

```text
projectId IN (project1, project2)
```

query the two facet partitions, union their document IDs, and intersect the result with other field constraints.

Do not use a DynamoDB `FilterExpression` as the main search filtering mechanism. DynamoDB applies it after reading the query page, so it does not reduce read capacity or the pre-filter 1 MB page limit.

### Query planner

Choose the most selective available seed using statistics:

1. A small `documentId IN (...)` set.
2. The smallest facet union.
3. The rarest text term.
4. A bounded oversampled text search when no selective condition exists.

The remaining clauses verify or reduce the seed set. This avoids materializing very large posting lists unnecessarily.

## Lexical retrieval and Top 5,000

Holding 5,000 IDs and scores in memory is inexpensive. The difficult part is proving that unread posting pages cannot contain a better result.

The initial implementation may use approximate candidate generation:

1. Read postings in descending approximate impact order.
2. Collect two to five times the requested lexical limit.
3. Apply Boolean and facet constraints.
4. Recalculate exact BM25 for surviving candidates.
5. Maintain a fixed-size top-K heap of 5,000 entries.

This behavior and its recall trade-off must be documented and benchmarked. A later exact implementation can add block-level score upper bounds and a Block-Max WAND-style stopping condition.

Unlike the reference DynamoSearch implementation, Pedro must follow `LastEvaluatedKey` when more posting pages are required. Silently ignoring all pages after the first is not suitable for a 5,000-result target.

Common terms require adaptive posting shards. Keep one shard for rare terms and increase the shard count for hot terms based on measured document frequency and traffic. Search all declared shards in parallel with bounded concurrency.

## Vector retrieval

Use S3 Vectors for the initial semantic backend:

- Store one vector key per document or searchable chunk.
- Store `documentId`, `projectId`, and other user filter values as filterable metadata.
- Request up to 100 results.
- Apply `$in`, `$and`, and `$or` metadata filters in the vector query.

Example filter:

```json
{
  "$and": [
    {
      "projectId": {
        "$in": ["project1", "project2"]
      }
    },
    {
      "hogeId": {
        "$in": ["hoge1"]
      }
    }
  ]
}
```

Adding a filterable field requires updating metadata for existing vectors before the field can be considered complete for historical queries.

Embedding generation remains outside the storage service. Source text updates must regenerate and replace the embedding.

## Hybrid ranking

Lexical BM25 scores and vector distances are not directly comparable. Reciprocal Rank Fusion is a suitable initial merge strategy:

```text
score(document) =
    lexicalWeight / (rrfConstant + lexicalRank)
  + vectorWeight  / (rrfConstant + vectorRank)
```

The candidate union contains at most 5,100 entries before deduplication. Domain-specific reranking can be added later.

## Indexing pipeline

Do not synchronously write all postings in the document ingestion request.

```text
Document write or upload
  -> DynamoDB Streams or S3 event
  -> SQS
  -> Indexer Lambda
       -> analyze text
       -> delete obsolete postings
       -> write new text postings
       -> update facet postings
       -> update forward index
       -> update statistics
       -> generate or update vector and metadata
```

The indexer must be idempotent. Every document carries a monotonically increasing `revision`, and stale events or stale backfill writes must not overwrite a newer indexed revision.

Bound document token count, field count, and multi-value cardinality so one document cannot create an unbounded number of posting writes. Split large write sets into batches and retry unprocessed DynamoDB operations with backoff.

## Schema changes

### Add a filterable field

1. Allocate a new stable `fieldId` and generation.
2. Store the field as `BACKFILLING`.
3. Begin indexing the field on new and updated documents.
4. Backfill only that field for historical documents from S3 or the source table.
5. Reconcile concurrent changes using document revisions.
6. Mark the field `ACTIVE` and allow it in queries.

### Change type or analyzer

Create a new field generation. Dual-write while backfilling, switch the active generation after validation, and retire the previous generation asynchronously. An analyzer change requires rebuilding text postings for that field.

### Remove a field

Mark the field `DISABLED` immediately for query validation. Stop new writes and delete the unreachable posting namespace asynchronously. Do not synchronously delete a large field index in the schema update request.

## Initial safety limits

Suggested starting limits:

```text
Filterable fields per index:       20
Searchable fields per index:        5
Values in one IN expression:      100
Values in one multi-value field:  100
Lexical result cap:              5000
Vector result cap:                100
Result page size:              50-100
```

Also limit:

- Field name and keyword value length
- Tokens and indexed bytes per document
- Boolean query depth and clause count
- Concurrent backfills per index and account
- Query fan-out and DynamoDB concurrency
- Search snapshot lifetime

## Why not use custom S3 lexical segments initially?

Immutable Lucene or Tantivy segments on S3 provide better compression and are a natural long-term design for large posting indexes. They also support bitmap filters and efficient top-K algorithms.

However, fast online search over those segments normally needs resident search workers with local caches. A Lambda-only implementation has less predictable cold latency, duplicates caches across concurrent executions, and must manage segment manifests, tombstones, and compaction.

DynamoDB is therefore the simpler initial online lexical backend for a Lambda-first product. S3 remains the durable source of truth and rebuild source.

Keep the lexical backend behind an interface so it can later be replaced by S3 plus Tantivy/Lucene and ECS search workers without changing the public schema or query API.

## Validation plan

Before treating the architecture as production-ready, benchmark representative workloads:

- Japanese analyzer output and relevance judgments
- Rare, common, AND, OR, and phrase queries
- Top 10, Top 100, and Top 5,000 retrieval
- Selective and broad `IN` filters
- Multiple simultaneous filter fields
- Hot-term partition behavior
- Posting storage and write amplification
- Index freshness and out-of-order stream events
- Schema backfill while documents are being updated
- Lambda cold and warm latency
- DynamoDB and S3 Vectors cost per search

The most important decision gate is whether approximate candidate retrieval provides acceptable recall at the target read cost. If it does not, implement bounded WAND-style retrieval or move lexical serving to a segment-based engine.

## References

- [DynamoSearch article](https://qiita.com/maruyamaworks/items/831ee49d98d92170bbec)
- [DynamoSearch documentation](https://maruyamaworks.github.io/dynamosearch/)
- [DynamoDB Query API](https://docs.aws.amazon.com/amazondynamodb/latest/APIReference/API_Query.html)
- [DynamoDB vector indexes](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/VectorSearch.html)
- [S3 Vectors](https://docs.aws.amazon.com/AmazonS3/latest/userguide/s3-vectors.html)
- [S3 Vectors metadata filtering](https://docs.aws.amazon.com/AmazonS3/latest/userguide/s3-vectors-metadata-filtering.html)
- [Quickwit architecture](https://quickwit.io/docs/main-branch/overview/architecture)
