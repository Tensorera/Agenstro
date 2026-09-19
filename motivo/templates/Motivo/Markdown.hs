{-# LANGUAGE OverloadedStrings #-}

-- | Shared block boundaries for section extraction and the escaped HTML reader.
-- This is deliberately a small Markdown projection, not a full Markdown parser.
module Motivo.Markdown (MarkdownBlock (..), markdownBlocks, blockLines) where

import Data.Maybe (isNothing)
import Data.Text (Text)
import qualified Data.Text as Text

data MarkdownBlock
  = OrdinaryLines [Text]
  | FencedCode Text [Text] (Maybe Text)
  deriving (Eq, Show)

data Fence = Fence Char Int

-- | Keep the original delimiters for source-preserving section extraction.
blockLines :: MarkdownBlock -> [Text]
blockLines (OrdinaryLines lines') = lines'
blockLines (FencedCode opening contents closing) = opening : contents ++ maybe [] pure closing

markdownBlocks :: Text -> [MarkdownBlock]
markdownBlocks = scan . Text.lines
  where
    scan [] = []
    scan lines'@(line : rest) = case openingFence line of
      Nothing ->
        let (ordinary, remaining) = span (isNothing . openingFence) lines'
        in OrdinaryLines ordinary : scan remaining
      Just fence ->
        let (contents, remaining) = break (closesFence fence) rest
        in case remaining of
          [] -> [FencedCode line contents Nothing]
          closing : after -> FencedCode line contents (Just closing) : scan after

-- Markdown permits at most three leading spaces before a fenced code block.
-- The character and minimum closing length belong to the opening delimiter.
fencePrefix :: Text -> Maybe (Fence, Text)
fencePrefix line = do
  let (indent, trimmed) = Text.span (== ' ') line
  (character, _) <- Text.uncons trimmed
  let (delimiter, suffix) = Text.span (== character) trimmed
      width = Text.length delimiter
  if Text.length indent <= 3 && character `elem` ['`', '~'] && width >= 3
    then Just (Fence character width, suffix)
    else Nothing

openingFence :: Text -> Maybe Fence
openingFence line = do
  (fence@(Fence character _), suffix) <- fencePrefix line
  if character == '`' && Text.any (== '`') suffix then Nothing else Just fence

closesFence :: Fence -> Text -> Bool
closesFence (Fence character width) line = case fencePrefix line of
  Just (Fence closingCharacter closingWidth, suffix) ->
    character == closingCharacter && closingWidth >= width && Text.all (`elem` [' ', '\t']) suffix
  Nothing -> False
