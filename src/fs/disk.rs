use std::path::Path;

use eyre::Result;
use floppy_disk::prelude::*;
use tracing::{debug, error, warn};

pub struct DiskDrive<
    'a,
    'b,
    F1: FloppyDisk<'a> + FloppyDiskUnixExt + Send + Sync + 'a,
    F2: FloppyDisk<'b> + FloppyDiskUnixExt + Send + Sync + 'b,
> where
    <F1 as FloppyDisk<'a>>::Permissions: FloppyUnixPermissions,
    <F1 as FloppyDisk<'a>>::Metadata: FloppyUnixMetadata,

    <F1 as FloppyDisk<'a>>::DirBuilder: Send,
    <F1 as FloppyDisk<'a>>::DirEntry: Send,
    <F1 as FloppyDisk<'a>>::File: Send,
    <F1 as FloppyDisk<'a>>::FileType: Send,
    <F1 as FloppyDisk<'a>>::Metadata: Send,
    <F1 as FloppyDisk<'a>>::OpenOptions: Send,
    <F1 as FloppyDisk<'a>>::Permissions: Send,
    <F1 as FloppyDisk<'a>>::ReadDir: Send,

    <F2 as FloppyDisk<'b>>::Permissions: FloppyUnixPermissions,
    <F2 as FloppyDisk<'b>>::Metadata: FloppyUnixMetadata,

    <F2 as FloppyDisk<'b>>::DirBuilder: Send,
    <F2 as FloppyDisk<'b>>::DirEntry: Send,
    <F2 as FloppyDisk<'b>>::File: Send,
    <F2 as FloppyDisk<'b>>::FileType: Send,
    <F2 as FloppyDisk<'b>>::Metadata: Send,
    <F2 as FloppyDisk<'b>>::OpenOptions: Send,
    <F2 as FloppyDisk<'b>>::Permissions: Send,
    <F2 as FloppyDisk<'b>>::ReadDir: Send,
{
    _f1: std::marker::PhantomData<&'a F1>,
    _f2: std::marker::PhantomData<&'b F2>,
}

impl<
        'a,
        'b,
        F1: FloppyDisk<'a> + FloppyDiskUnixExt + Send + Sync + 'a,
        F2: FloppyDisk<'b> + FloppyDiskUnixExt + Send + Sync + 'b,
    > DiskDrive<'a, 'b, F1, F2>
where
    <F1 as FloppyDisk<'a>>::Permissions: FloppyUnixPermissions,
    <F1 as FloppyDisk<'a>>::Metadata: FloppyUnixMetadata,
    <F2 as FloppyDisk<'b>>::Permissions: FloppyUnixPermissions,
    <F2 as FloppyDisk<'b>>::Metadata: FloppyUnixMetadata,
{
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            _f1: std::marker::PhantomData,
            _f2: std::marker::PhantomData,
        }
    }

    pub async fn copy_between(&self, src: &'a F1, dest: &'b F2) -> Result<()> {
        let paths = nyoom::walk_ordered(src, Path::new("/")).await?;
        for src_path in paths {
            let dest_path = Path::new("/");
            debug!("copy {} -> {}", src_path.display(), dest_path.display());
            let metadata = <F1 as FloppyDisk<'a>>::metadata(src, &src_path).await?;
            let file_type = metadata.file_type();
            if file_type.is_dir() {
                Self::copy_dir_to_memfs(src, dest, &src_path, dest_path).await?;
            } else if file_type.is_file() {
                Self::copy_file_to_memfs(src, dest, &src_path, dest_path).await?;
            } else if file_type.is_symlink() {
                Self::add_symlink_to_memfs(src, dest, &src_path, dest_path).await?;
            } else {
                error!("unknown file type for source path {src_path:?}");
            }
        }

        Ok(())
    }

    async fn copy_file_to_memfs(
        src: &'a F1,
        dest: &'b F2,
        src_path: &Path,
        dest_path: &Path,
    ) -> Result<()> {
        debug!("creating file {dest_path:?}");
        if let Some(memfs_parent) = dest_path.parent() {
            dest.create_dir_all(memfs_parent).await?;
        }

        let mut src_handle: <F1 as FloppyDisk>::File = <F1::OpenOptions>::new()
            .read(true)
            .open(src, src_path)
            .await?;
        {
            let dest_metadata = <F2 as FloppyDisk>::metadata(dest, dest_path).await;
            let dest_handle = <F2::OpenOptions>::new();
            let dest_handle = dest_handle.create(true).write(true);

            // if dest metadata doesn't exist, just copy directly
            if dest_metadata.is_err() {
                debug!("dest {dest_path:?} doesn't exist, copying directly!");
                let mut dest_handle: <F2 as FloppyDisk>::File =
                    dest_handle.open(dest, dest_path).await?;

                tokio::io::copy(&mut src_handle, &mut dest_handle).await?;
                return Ok(());
            }

            // if dest exists and is a dir, copy into it
            let dest_metadata = dest_metadata?;
            if dest_metadata.is_dir() {
                debug!("copying into dir {dest_path:?}");
                let dest_path = dest_path.join(Path::new(src_path.file_name().unwrap()));
                debug!("target path = {dest_path:?}");
                let mut dest_handle: <F2 as FloppyDisk>::File =
                    dest_handle.open(dest, &dest_path).await?;

                tokio::io::copy(&mut src_handle, &mut dest_handle).await?;
                // copy permissions
                let src_metadata = src_handle.metadata().await?;
                let src_permissions = src_metadata.permissions();
                let mode = <<F1 as FloppyDisk<'_>>::Permissions as FloppyUnixPermissions>::mode(
                    &src_permissions,
                );

                let permissions = <<F2 as FloppyDisk>::Permissions>::from_mode(mode);
                let uid = src_metadata.uid()?;
                let gid = src_metadata.gid()?;

                <F2 as FloppyDiskUnixExt>::chown(dest, &dest_path, uid, gid).await?;
                <F2 as FloppyDisk>::set_permissions(dest, &dest_path, permissions).await?;

                return Ok(());
            }

            // if dest exists and is a file, copy into it
            if dest_metadata.is_file() {
                warn!("overwriting dest file {dest_path:?}");
                let mut dest_handle: <F2 as FloppyDisk>::File =
                    dest_handle.open(dest, dest_path).await?;

                tokio::io::copy(&mut src_handle, &mut dest_handle).await?;
                return Ok(());
            }

            // if dest exists and is a symlink, log error and return
            if dest_metadata.is_symlink() {
                warn!("dest path {dest_path:?} is a symlink, skipping copy!");
                return Ok(());
            }
        }

        let src_metadata = src_handle.metadata().await?;
        let src_permissions = src_metadata.permissions();
        let mode =
            <<F1 as FloppyDisk<'_>>::Permissions as FloppyUnixPermissions>::mode(&src_permissions);
        let permissions = <<F2 as FloppyDisk>::Permissions>::from_mode(mode);
        let uid = src_metadata.uid()?;
        let gid = src_metadata.gid()?;
        <F2 as FloppyDiskUnixExt>::chown(dest, dest_path, uid, gid).await?;
        <F2 as FloppyDisk>::set_permissions(dest, dest_path, permissions).await?;

        Ok(())
    }

    async fn copy_dir_to_memfs(
        src: &'a F1,
        dest: &'b F2,
        src_path: &Path,
        dest_path: &Path,
    ) -> Result<()> {
        dest.create_dir_all(dest_path).await?;

        let src_metadata = src.metadata(src_path).await?;
        let mode = src_metadata.permissions().mode();
        let permissions = <F2 as FloppyDisk>::Permissions::from_mode(mode);
        dest.set_permissions(dest_path, permissions).await?;
        dest.chown(dest_path, src_metadata.uid()?, src_metadata.gid()?)
            .await?;

        Ok(())
    }

    async fn add_symlink_to_memfs(
        src: &F1,
        dest: &F2,
        path: &Path,
        memfs_path: &Path,
    ) -> Result<()> {
        let link = src.read_link(path).await?;
        debug!("linking {memfs_path:?} to {link:?}");
        dest.symlink(link, memfs_path.into()).await?;

        Ok(())
    }
}
