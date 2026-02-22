use worker::*;

/// Store a blob in R2 and return its size.
pub async fn put_blob(bucket: &Bucket, key: &str, data: Vec<u8>) -> Result<u64> {
    let size = data.len() as u64;
    bucket.put(key, data).execute().await?;
    Ok(size)
}

/// Retrieve a blob from R2. Returns None if not found.
pub async fn get_blob(bucket: &Bucket, key: &str) -> Result<Option<Vec<u8>>> {
    let obj = bucket.get(key).execute().await?;
    match obj {
        Some(obj) => match obj.body() {
            Some(body) => {
                let bytes = body.bytes().await?;
                Ok(Some(bytes))
            }
            None => Ok(Some(Vec::new())),
        },
        None => Ok(None),
    }
}

/// Delete a blob from R2.
pub async fn delete_blob(bucket: &Bucket, key: &str) -> Result<()> {
    bucket.delete(key).await
}
