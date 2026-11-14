import { useLocalSearchParams } from 'expo-router'

import { DriveFolder } from '../../../components/drive/DriveFolder'

/** A Drive folder. */
export default function DriveFolderScreen() {
  const { id } = useLocalSearchParams<{ id: string }>()
  return <DriveFolder key={id} folderId={id} />
}
