use anyhow::Result;

/// Carga el XML desde una URL y retorna el contenido como String.
pub async fn load_xml_from_url(url: &str) -> Result<String> {
    let response = reqwest::get(url).await?;
    let response = response.error_for_status()?;
    let text = response.text().await?;
    Ok(text)
}
