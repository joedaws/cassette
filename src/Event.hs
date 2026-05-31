{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE RankNTypes #-}
{-# LANGUAGE TemplateHaskell #-}
{-# OPTIONS_GHC -Wno-incomplete-uni-patterns #-}

module Event
  ( appEvent,
    AppEvent (..),
    Name (..),
    St (..),
    tapes,
    focusIdx,
    termHeight,
    statusMsg,
    timerSecs,
    reelRotation,
    wordGoal,
    modifyFocusedTape,
    addTapeToSt,
    focusNextSt,
    focusPrevSt,
    modifyFocusedTapeSt,
    tickTimer,
    advanceReel,
    calcMaxTapes,
    initialState,
    wordCountTape,
    keyQuit,
    keyNextTape,
    keyPrevTape,
    keyAddTape,
    modAddTape,
  )
where

import qualified Brick.Main as M
import qualified Brick.Types as T
import Data.Char (isPrint)
import qualified Data.Text as DT
import Deck (tapeRows, tapeWidth)
import qualified Graphics.Vty as V
import Lens.Micro ((.~), (%~))
import Lens.Micro.Mtl
import Lens.Micro.TH
import Tape

data AppEvent = Tick
  deriving (Eq, Show)

newtype Name = Edit Int
  deriving (Ord, Show, Eq)

data St = St
  { _tapes        :: [Tape],
    _focusIdx     :: Int,
    _termHeight   :: Int,
    _statusMsg    :: Maybe DT.Text,
    _timerSecs    :: Maybe Int,
    _reelRotation :: Int,
    _wordGoal     :: Maybe Int
  }

makeLenses ''St

-- Key binding constants
keyQuit        :: V.Key;  keyQuit        = V.KEsc
keyNextTape    :: V.Key;  keyNextTape    = V.KChar '\t'
keyPrevTape    :: V.Key;  keyPrevTape    = V.KBackTab
keyAddTape     :: V.Key;  keyAddTape     = V.KChar 'n'
modAddTape     :: [V.Modifier]; modAddTape = [V.MCtrl]
keyBackspace   :: V.Key;  keyBackspace   = V.KBS
keyDelete      :: V.Key;  keyDelete      = V.KDel
keyCursorLeft  :: V.Key;  keyCursorLeft  = V.KLeft
keyCursorRight :: V.Key;  keyCursorRight = V.KRight

-- Vty 6 / vty-unix may report Ctrl+N as raw byte 0x0E with no modifiers
-- rather than KChar 'n' [MCtrl], depending on terminal/platform.
isAddTapeKey :: V.Key -> [V.Modifier] -> Bool
isAddTapeKey k ms = (k == keyAddTape && ms == modAddTape)
                 || (k == V.KChar '\x0e' && null ms)

-- Screen capacity: top separators per tape + bottom sep + reel stats + status + help
uiOverheadRows :: Int
uiOverheadRows = 4

calcMaxTapes :: Int -> Int
calcMaxTapes termH = max 1 ((termH - uiOverheadRows) `div` tapeRows)

-- | Word count for a tape
wordCountTape :: Tape -> Int
wordCountTape = length . DT.words . tapeText

-- Pure state helpers
addTapeToSt :: St -> St
addTapeToSt st
  | length (_tapes st) >= calcMaxTapes (_termHeight st) =
      st { _statusMsg = Just (DT.pack "No more vertical space for additional tapes.") }
  | otherwise =
      let tw = case _tapes st of
                 (t:_) -> _width t
                 []    -> 11
          newTape = (initTape "" 0) { _width = tw }
      in  st { _tapes     = _tapes st ++ [newTape],
               _focusIdx  = length (_tapes st),
               _statusMsg = Nothing }

focusNextSt :: St -> St
focusNextSt st =
  st { _focusIdx  = (_focusIdx st + 1) `mod` max 1 (length (_tapes st)),
       _statusMsg = Nothing }

focusPrevSt :: St -> St
focusPrevSt st =
  let n = length (_tapes st)
  in  st { _focusIdx  = (_focusIdx st - 1 + n) `mod` max 1 n,
           _statusMsg = Nothing }

modifyFocusedTapeSt :: (Tape -> Tape) -> St -> St
modifyFocusedTapeSt f st
  | idx < 0 || idx >= length ts = st
  | otherwise =
      let (before, tape : after) = splitAt idx ts
      in  st { _tapes = before ++ [f tape] ++ after }
  where
    idx = _focusIdx st
    ts  = _tapes st

-- Monadic wrapper
modifyFocusedTape :: (Tape -> Tape) -> T.EventM Name St ()
modifyFocusedTape f = T.modify (modifyFocusedTapeSt f)

-- | Advance the reel animation by one frame
advanceReel :: St -> St
advanceReel = reelRotation %~ (\r -> succ r `mod` 4)

initialState :: St
initialState = St
  { _tapes        = [initTape "" 0],
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
appEvent (T.AppEvent Tick)                                  = T.modify tickTimer
appEvent (T.VtyEvent (V.EvKey k [])) | k == keyQuit        = M.halt
appEvent (T.VtyEvent (V.EvKey k [])) | k == keyNextTape    = T.modify focusNextSt
appEvent (T.VtyEvent (V.EvKey k [])) | k == keyPrevTape    = T.modify focusPrevSt
appEvent (T.VtyEvent (V.EvKey k ms)) | k == keyAddTape
                                     , ms == modAddTape    = T.modify addTapeToSt
appEvent (T.VtyEvent (V.EvKey k [])) | k == keyBackspace   = do
  modifyFocusedTape backspace
  T.modify advanceReel
appEvent (T.VtyEvent (V.EvKey k [])) | k == keyDelete      = do
  modifyFocusedTape delete
  T.modify advanceReel
appEvent (T.VtyEvent (V.EvKey k [])) | k == keyCursorLeft  = modifyFocusedTape rewind
appEvent (T.VtyEvent (V.EvKey k [])) | k == keyCursorRight = modifyFocusedTape forward
appEvent (T.VtyEvent (V.EvResize w h)) = do
  termHeight .= h
  let tw = tapeWidth w
  tapes %= fmap (width .~ tw)
appEvent (T.VtyEvent (V.EvKey (V.KChar c) []))
  | isPrint c = do
    modifyFocusedTape (`insert` c)
    T.modify advanceReel
appEvent _ = return ()
