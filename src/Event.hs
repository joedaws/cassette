{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE RankNTypes #-}
{-# LANGUAGE TemplateHaskell #-}
{-# OPTIONS_GHC -Wno-incomplete-uni-patterns #-}

module Event
  ( appEvent,
    AppEvent (..),
    Name (..),
    St (..),
    cassettes,
    focusIdx,
    termHeight,
    statusMsg,
    timerSecs,
    reelRotation,
    wordGoal,
    modifyFocusedCassette,
    addCassetteToSt,
    focusNextSt,
    focusPrevSt,
    modifyFocusedCassetteSt,
    tickTimer,
    advanceReel,
    calcMaxCassettes,
    initialState,
    wordCountCassette,
    keyQuit,
    keyNextCassette,
    keyPrevCassette,
    keyAddCassette,
    modAddCassette,
  )
where

import qualified Brick.Main as M
import qualified Brick.Types as T
import Data.Char (isPrint)
import qualified Data.Text as DT
import Deck (cassetteRows, cassetteWidth)
import qualified Graphics.Vty as V
import Lens.Micro ((.~), (%~))
import Lens.Micro.Mtl
import Lens.Micro.TH
import Cassette

data AppEvent = Tick
  deriving (Eq, Show)

newtype Name = Edit Int
  deriving (Ord, Show, Eq)

data St = St
  { _cassettes    :: [Cassette],
    _focusIdx     :: Int,
    _termHeight   :: Int,
    _statusMsg    :: Maybe DT.Text,
    _timerSecs    :: Maybe Int,
    _reelRotation :: Int,
    _wordGoal     :: Maybe Int
  }

makeLenses ''St

-- Key binding constants
keyQuit           :: V.Key;  keyQuit           = V.KEsc
keyNextCassette   :: V.Key;  keyNextCassette   = V.KChar '\t'
keyPrevCassette   :: V.Key;  keyPrevCassette   = V.KBackTab
keyAddCassette    :: V.Key;  keyAddCassette    = V.KChar 'n'
modAddCassette    :: [V.Modifier]; modAddCassette = [V.MCtrl]
keyBackspace      :: V.Key;  keyBackspace      = V.KBS
keyDelete         :: V.Key;  keyDelete         = V.KDel
keyCursorLeft     :: V.Key;  keyCursorLeft     = V.KLeft
keyCursorRight    :: V.Key;  keyCursorRight    = V.KRight

-- Vty 6 / vty-unix may report Ctrl+N as raw byte 0x0E with no modifiers
-- rather than KChar 'n' [MCtrl], depending on terminal/platform.
isAddCassetteKey :: V.Key -> [V.Modifier] -> Bool
isAddCassetteKey k ms = (k == keyAddCassette && ms == modAddCassette)
                     || (k == V.KChar '\x0e' && null ms)

-- Screen capacity: top separators per cassette + bottom sep + reel stats + status + help
uiOverheadRows :: Int
uiOverheadRows = 4

calcMaxCassettes :: Int -> Int
calcMaxCassettes termH = max 1 ((termH - uiOverheadRows) `div` cassetteRows)

-- | Word count for a cassette
wordCountCassette :: Cassette -> Int
wordCountCassette = length . DT.words . cassetteText

-- Pure state helpers
addCassetteToSt :: St -> St
addCassetteToSt st
  | length (_cassettes st) >= calcMaxCassettes (_termHeight st) =
      st { _statusMsg = Just (DT.pack "No more vertical space for additional cassettes.") }
  | otherwise =
      let tw = case _cassettes st of
                 (t:_) -> _width t
                 []    -> 11
          newCassette = (initCassette "" 0) { _width = tw }
      in  st { _cassettes  = _cassettes st ++ [newCassette],
               _focusIdx   = length (_cassettes st),
               _statusMsg  = Nothing }

focusNextSt :: St -> St
focusNextSt st =
  st { _focusIdx  = (_focusIdx st + 1) `mod` max 1 (length (_cassettes st)),
       _statusMsg = Nothing }

focusPrevSt :: St -> St
focusPrevSt st =
  let n = length (_cassettes st)
  in  st { _focusIdx  = (_focusIdx st - 1 + n) `mod` max 1 n,
           _statusMsg = Nothing }

modifyFocusedCassetteSt :: (Cassette -> Cassette) -> St -> St
modifyFocusedCassetteSt f st
  | idx < 0 || idx >= length cs = st
  | otherwise =
      let (before, cassette : after) = splitAt idx cs
      in  st { _cassettes = before ++ [f cassette] ++ after }
  where
    idx = _focusIdx st
    cs  = _cassettes st

-- Monadic wrapper
modifyFocusedCassette :: (Cassette -> Cassette) -> T.EventM Name St ()
modifyFocusedCassette f = T.modify (modifyFocusedCassetteSt f)

-- | Advance the reel animation by one frame
advanceReel :: St -> St
advanceReel = reelRotation %~ (\r -> succ r `mod` 4)

initialState :: St
initialState = St
  { _cassettes    = [initCassette "" 0],
    _focusIdx     = 0,
    _termHeight   = 24,
    _statusMsg    = Nothing,
    _timerSecs    = Nothing,
    _reelRotation = 0,
    _wordGoal     = Nothing
  }

tickTimer :: St -> St
tickTimer st =
  case _timerSecs st of
    Nothing -> st
    Just 0  -> st
    Just n  -> st { _timerSecs = Just (n - 1) }

appEvent :: T.BrickEvent Name AppEvent -> T.EventM Name St ()
appEvent (T.AppEvent Tick)                                          = T.modify tickTimer
appEvent (T.VtyEvent (V.EvKey k [])) | k == keyQuit                = M.halt
appEvent (T.VtyEvent (V.EvKey k [])) | k == keyNextCassette        = T.modify focusNextSt
appEvent (T.VtyEvent (V.EvKey k [])) | k == keyPrevCassette        = T.modify focusPrevSt
appEvent (T.VtyEvent (V.EvKey k ms)) | k == keyAddCassette
                                     , ms == modAddCassette         = T.modify addCassetteToSt
appEvent (T.VtyEvent (V.EvKey k [])) | k == keyBackspace            = do
  modifyFocusedCassette backspace
  T.modify advanceReel
appEvent (T.VtyEvent (V.EvKey k [])) | k == keyDelete               = do
  modifyFocusedCassette delete
  T.modify advanceReel
appEvent (T.VtyEvent (V.EvKey k [])) | k == keyCursorLeft           = modifyFocusedCassette rewind
appEvent (T.VtyEvent (V.EvKey k [])) | k == keyCursorRight          = modifyFocusedCassette forward
appEvent (T.VtyEvent (V.EvResize w h)) = do
  termHeight .= h
  let tw = cassetteWidth w
  cassettes %= fmap (width .~ tw)
appEvent (T.VtyEvent (V.EvKey (V.KChar c) []))
  | isPrint c = do
    modifyFocusedCassette (`insert` c)
    T.modify advanceReel
appEvent _ = return ()
