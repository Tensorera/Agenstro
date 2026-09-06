{-# LANGUAGE OverloadedStrings #-}

-- | A deliberately small, escaped Markdown projection. Raw HTML is always text.
module Motivo.Html (escapeHtml, renderMarkdown, tag, renderPage) where

import Data.Text (Text)
import qualified Data.Text as Text

escapeHtml :: Text -> Text
escapeHtml = Text.concatMap escape
  where
    escape '&' = "&amp;"
    escape '<' = "&lt;"
    escape '>' = "&gt;"
    escape '"' = "&quot;"
    escape '\'' = "&#39;"
    escape character = Text.singleton character

tag :: Text -> Text -> Text
tag name body = "<" <> name <> ">" <> body <> "</" <> name <> ">"

-- | Do not turn arbitrary source text into URLs, attributes, HTML or scripts.
-- Generated report/artifact links are constructed separately from safe tokens.
renderMarkdown :: Text -> Text
renderMarkdown = Text.concat . renderLines . Text.lines
  where
    renderLines [] = []
    renderLines (line : rest)
      | "```" `Text.isPrefixOf` line =
          let (code, remaining) = break (Text.isPrefixOf "```") rest
          in tag "pre" (tag "code" (escapeHtml (Text.unlines code))) : renderLines (drop 1 remaining)
      | Text.null (Text.strip line) = renderLines rest
      | "### " `Text.isPrefixOf` line = tag "h3" (escapeHtml (Text.drop 4 line)) : renderLines rest
      | "## " `Text.isPrefixOf` line = tag "h2" (escapeHtml (Text.drop 3 line)) : renderLines rest
      | "# " `Text.isPrefixOf` line = tag "h2" (escapeHtml (Text.drop 2 line)) : renderLines rest
      | isBullet line =
          let (items, remaining) = span isBullet (line : rest)
          in tag "ul" (Text.concat (map (tag "li" . escapeHtml . Text.drop 2) items)) : renderLines remaining
      | "|" `Text.isPrefixOf` Text.strip line =
          let (rows, remaining) = span (Text.isPrefixOf "|" . Text.strip) (line : rest)
          in renderTable rows : renderLines remaining
      | otherwise = tag "p" (escapeHtml line) : renderLines rest
    isBullet line = "- " `Text.isPrefixOf` line || "* " `Text.isPrefixOf` line
    cells = map Text.strip . Text.splitOn "|" . Text.dropAround (== '|') . Text.strip
    divider row = let values = cells row in not (null values) && all (Text.all (`elem` (" :-" :: String))) values
    renderTable rows = "<div class=\"table-scroll\"><table>" <> Text.concat (map renderRow (filter (not . divider) rows)) <> "</table></div>"
    renderRow = tag "tr" . Text.concat . map (tag "td" . escapeHtml) . cells

renderPage :: Text -> Text -> Either String Text
renderPage template body =
  case Text.splitOn "<!-- MOTIVO_CONTENT -->" template of
    [before, after] -> Right (before <> body <> after)
    _ -> Left "Motivo report template must contain exactly one <!-- MOTIVO_CONTENT --> marker"
